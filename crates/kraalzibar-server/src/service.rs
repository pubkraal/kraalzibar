use std::sync::Arc;
use std::time::Duration;

use kraalzibar_core::engine::{
    CheckEngine, CheckRequest, CheckResult, EngineConfig, ExpandEngine, ExpandRequest, ExpandTree,
};
use kraalzibar_core::schema::types::Schema;
use kraalzibar_core::schema::{
    SchemaLimits, detect_breaking_changes, parse_schema, validate_schema_limits,
};
use kraalzibar_core::tuple::{SnapshotToken, SubjectRef, TenantId, Tuple, TupleFilter, TupleWrite};
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::adapter::StoreTupleReader;
use crate::audit;
use crate::config::CacheConfig;
use crate::error::ApiError;
use crate::metrics::Metrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consistency {
    FullConsistency,
    MinimizeLatency,
    AtLeastAsFresh(SnapshotToken),
    AtExactSnapshot(SnapshotToken),
}

#[derive(Debug)]
pub struct CheckPermissionInput {
    pub object_type: String,
    pub object_id: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    pub consistency: Consistency,
}

#[derive(Debug)]
pub struct CheckPermissionOutput {
    pub allowed: bool,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct ExpandPermissionInput {
    pub object_type: String,
    pub object_id: String,
    pub permission: String,
    pub consistency: Consistency,
}

#[derive(Debug)]
pub struct LookupSubjectsInput {
    pub object_type: String,
    pub object_id: String,
    pub permission: String,
    pub subject_type: String,
    pub consistency: Consistency,
}

#[derive(Debug)]
pub struct LookupResourcesInput {
    pub resource_type: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    pub consistency: Consistency,
    pub limit: Option<usize>,
}

#[derive(Debug)]
pub struct ExpandPermissionOutput {
    pub tree: ExpandTree,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct LookupResourcesOutput {
    pub resource_ids: Vec<String>,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct LookupSubjectsOutput {
    pub subjects: Vec<SubjectRef>,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct ReadRelationshipsOutput {
    pub tuples: Vec<Tuple>,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct WriteSchemaOutput {
    pub breaking_changes_overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CheckCacheKey {
    tenant_id: TenantId,
    object_type: String,
    object_id: String,
    permission: String,
    subject_type: String,
    subject_id: String,
    snapshot: SnapshotToken,
}

pub struct AuthzService<F: StoreFactory> {
    factory: Arc<F>,
    engine_config: EngineConfig,
    schema_limits: SchemaLimits,
    schema_cache: moka::future::Cache<TenantId, Arc<Schema>>,
    check_cache: moka::future::Cache<CheckCacheKey, bool>,
    metrics: Option<Arc<Metrics>>,
}

impl<F: StoreFactory> AuthzService<F>
where
    F::Store: RelationshipStore + SchemaStore,
{
    pub fn new(factory: Arc<F>, engine_config: EngineConfig, schema_limits: SchemaLimits) -> Self {
        Self::with_cache_config(
            factory,
            engine_config,
            schema_limits,
            CacheConfig::default(),
        )
    }

    pub fn with_cache_config(
        factory: Arc<F>,
        engine_config: EngineConfig,
        schema_limits: SchemaLimits,
        cache_config: CacheConfig,
    ) -> Self {
        let schema_cache = moka::future::Cache::builder()
            .max_capacity(cache_config.schema_cache_capacity)
            .time_to_live(Duration::from_secs(cache_config.schema_cache_ttl_seconds))
            .build();
        let check_cache = moka::future::Cache::builder()
            .max_capacity(cache_config.check_cache_capacity)
            .time_to_live(Duration::from_secs(cache_config.check_cache_ttl_seconds))
            .build();
        Self {
            factory,
            engine_config,
            schema_limits,
            schema_cache,
            check_cache,
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    #[tracing::instrument(skip(self, input), fields(%tenant_id, object_type = %input.object_type, permission = %input.permission))]
    pub async fn check_permission(
        &self,
        tenant_id: &TenantId,
        input: CheckPermissionInput,
    ) -> Result<CheckPermissionOutput, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let store = Arc::new(store);

        let snapshot = self.resolve_snapshot(&*store, input.consistency).await?;

        // Only cache when we have an explicit snapshot token
        if let Some(snap) = snapshot {
            let cache_key = CheckCacheKey {
                tenant_id: tenant_id.clone(),
                object_type: input.object_type.clone(),
                object_id: input.object_id.clone(),
                permission: input.permission.clone(),
                subject_type: input.subject_type.clone(),
                subject_id: input.subject_id.clone(),
                snapshot: snap,
            };

            if let Some(cached_allowed) = self.check_cache.get(&cache_key).await {
                if let Some(m) = &self.metrics {
                    m.record_check_cache_hit();
                }
                return Ok(CheckPermissionOutput {
                    allowed: cached_allowed,
                    snapshot,
                });
            }

            if let Some(m) = &self.metrics {
                m.record_check_cache_miss();
            }
            let schema = self.load_schema_cached(tenant_id, &*store).await?;
            let reader = StoreTupleReader::new(Arc::clone(&store));
            let engine = CheckEngine::new(Arc::new(reader), schema, self.engine_config.clone());

            let request = CheckRequest {
                object_type: input.object_type,
                object_id: input.object_id,
                permission: input.permission,
                subject_type: input.subject_type,
                subject_id: input.subject_id,
                snapshot,
            };

            let result: CheckResult = engine.check(&request).await?;
            self.check_cache.insert(cache_key, result.allowed).await;
            Ok(CheckPermissionOutput {
                allowed: result.allowed,
                snapshot,
            })
        } else {
            // No snapshot — don't cache
            let schema = self.load_schema_cached(tenant_id, &*store).await?;
            let reader = StoreTupleReader::new(Arc::clone(&store));
            let engine = CheckEngine::new(Arc::new(reader), schema, self.engine_config.clone());

            let request = CheckRequest {
                object_type: input.object_type,
                object_id: input.object_id,
                permission: input.permission,
                subject_type: input.subject_type,
                subject_id: input.subject_id,
                snapshot,
            };

            let result: CheckResult = engine.check(&request).await?;
            Ok(CheckPermissionOutput {
                allowed: result.allowed,
                snapshot,
            })
        }
    }

    #[tracing::instrument(skip(self, input), fields(%tenant_id, object_type = %input.object_type, permission = %input.permission))]
    pub async fn expand_permission_tree(
        &self,
        tenant_id: &TenantId,
        input: ExpandPermissionInput,
    ) -> Result<ExpandPermissionOutput, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let store = Arc::new(store);

        let snapshot = self.resolve_snapshot(&*store, input.consistency).await?;
        let schema = self.load_schema_cached(tenant_id, &*store).await?;

        let reader = StoreTupleReader::new(Arc::clone(&store));
        let engine = ExpandEngine::new(Arc::new(reader), schema, self.engine_config.clone());

        let request = ExpandRequest {
            object_type: input.object_type,
            object_id: input.object_id,
            permission: input.permission,
            snapshot,
        };

        let tree = engine.expand(&request).await?;
        Ok(ExpandPermissionOutput { tree, snapshot })
    }

    #[tracing::instrument(skip(self, writes, deletes), fields(%tenant_id))]
    pub async fn write_relationships(
        &self,
        tenant_id: &TenantId,
        writes: &[TupleWrite],
        deletes: &[TupleFilter],
    ) -> Result<SnapshotToken, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let token = store.write(writes, deletes).await?;
        self.check_cache.invalidate_all();
        audit::audit_relationship_write(tenant_id, writes.len(), deletes.len(), &token);
        Ok(token)
    }

    #[tracing::instrument(skip(self, filter), fields(%tenant_id))]
    pub async fn read_relationships(
        &self,
        tenant_id: &TenantId,
        filter: &TupleFilter,
        consistency: Consistency,
        limit: Option<usize>,
    ) -> Result<ReadRelationshipsOutput, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let snapshot = self.resolve_snapshot(&store, consistency).await?;
        let tuples = store.read(filter, snapshot, limit).await?;
        Ok(ReadRelationshipsOutput { tuples, snapshot })
    }

    #[tracing::instrument(skip(self, definition), fields(%tenant_id))]
    pub async fn write_schema(
        &self,
        tenant_id: &TenantId,
        definition: &str,
        force: bool,
    ) -> Result<WriteSchemaOutput, ApiError> {
        let new_schema = parse_schema(definition)?;

        if let Err(errors) = validate_schema_limits(&new_schema, &self.schema_limits) {
            return Err(ApiError::Validation(errors));
        }

        let store = self.factory.for_tenant(tenant_id);
        let existing_text = store.read_schema().await?;

        let mut breaking_changes_overridden = false;

        if let Some(ref old_text) = existing_text {
            let old_schema = parse_schema(old_text)?;
            let breaking = detect_breaking_changes(&old_schema, &new_schema);

            if !breaking.is_empty() && !force {
                return Err(ApiError::BreakingChanges(breaking));
            }
            breaking_changes_overridden = !breaking.is_empty();
        }

        store.write_schema(definition).await?;
        self.schema_cache.invalidate(tenant_id).await;
        self.check_cache.invalidate_all();

        // Schema writes don't produce a snapshot token (schemas aren't versioned
        // in the tuple store), so the audit event has no token.
        audit::audit_schema_write(tenant_id, force, breaking_changes_overridden, None);

        Ok(WriteSchemaOutput {
            breaking_changes_overridden,
        })
    }

    #[tracing::instrument(skip(self), fields(%tenant_id))]
    pub async fn read_schema(&self, tenant_id: &TenantId) -> Result<Option<String>, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        Ok(store.read_schema().await?)
    }

    #[tracing::instrument(skip(self, input), fields(%tenant_id, object_type = %input.object_type, permission = %input.permission, subject_type = %input.subject_type))]
    pub async fn lookup_subjects(
        &self,
        tenant_id: &TenantId,
        input: LookupSubjectsInput,
    ) -> Result<LookupSubjectsOutput, ApiError> {
        let expand_output = self
            .expand_permission_tree(
                tenant_id,
                ExpandPermissionInput {
                    object_type: input.object_type,
                    object_id: input.object_id,
                    permission: input.permission,
                    consistency: input.consistency,
                },
            )
            .await?;

        let mut subjects = Vec::new();
        collect_subjects(&expand_output.tree, &input.subject_type, &mut subjects);
        subjects.sort_by(|a, b| {
            a.subject_type
                .cmp(&b.subject_type)
                .then(a.subject_id.cmp(&b.subject_id))
        });
        subjects.dedup();
        Ok(LookupSubjectsOutput {
            subjects,
            snapshot: expand_output.snapshot,
        })
    }

    #[tracing::instrument(skip(self, input), fields(%tenant_id, resource_type = %input.resource_type, permission = %input.permission))]
    pub async fn lookup_resources(
        &self,
        tenant_id: &TenantId,
        input: LookupResourcesInput,
    ) -> Result<LookupResourcesOutput, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let store = Arc::new(store);
        let snapshot = self.resolve_snapshot(&*store, input.consistency).await?;

        let filter = TupleFilter {
            object_type: Some(input.resource_type.clone()),
            ..Default::default()
        };
        let tuples = store.read(&filter, snapshot, None).await?;

        let mut object_ids: Vec<String> =
            tuples.iter().map(|t| t.object.object_id.clone()).collect();
        object_ids.sort();
        object_ids.dedup();

        let schema = self.load_schema_cached(tenant_id, &*store).await?;

        let reader = StoreTupleReader::new(Arc::clone(&store));
        let engine = CheckEngine::new(Arc::new(reader), schema, self.engine_config.clone());

        let limit = input.limit.unwrap_or(1000);
        let mut results = Vec::new();

        for object_id in &object_ids {
            if results.len() >= limit {
                break;
            }

            let request = CheckRequest {
                object_type: input.resource_type.clone(),
                object_id: object_id.clone(),
                permission: input.permission.clone(),
                subject_type: input.subject_type.clone(),
                subject_id: input.subject_id.clone(),
                snapshot,
            };

            let check_result = engine.check(&request).await?;
            if check_result.allowed {
                results.push(object_id.clone());
            }
        }

        Ok(LookupResourcesOutput {
            resource_ids: results,
            snapshot,
        })
    }

    async fn load_schema_cached(
        &self,
        tenant_id: &TenantId,
        store: &impl SchemaStore,
    ) -> Result<Arc<Schema>, ApiError> {
        if let Some(cached) = self.schema_cache.get(tenant_id).await {
            if let Some(m) = &self.metrics {
                m.record_schema_cache_hit();
            }
            return Ok(cached);
        }

        if let Some(m) = &self.metrics {
            m.record_schema_cache_miss();
        }
        let schema_text = store.read_schema().await?.ok_or(ApiError::SchemaNotFound)?;
        let schema = Arc::new(parse_schema(&schema_text)?);
        self.schema_cache
            .insert(tenant_id.clone(), Arc::clone(&schema))
            .await;
        Ok(schema)
    }

    async fn resolve_snapshot<S: RelationshipStore>(
        &self,
        store: &S,
        consistency: Consistency,
    ) -> Result<Option<SnapshotToken>, ApiError> {
        match consistency {
            Consistency::FullConsistency => {
                let token = store.snapshot().await?;
                Ok(Some(token))
            }
            Consistency::MinimizeLatency => Ok(None),
            Consistency::AtLeastAsFresh(token) | Consistency::AtExactSnapshot(token) => {
                Ok(Some(token))
            }
        }
    }
}

fn collect_subjects(tree: &ExpandTree, subject_type: &str, out: &mut Vec<SubjectRef>) {
    match tree {
        ExpandTree::Leaf { subject } => {
            if subject.subject_type == subject_type {
                out.push(subject.clone());
            }
        }
        ExpandTree::This { subjects, .. } => {
            for child in subjects {
                collect_subjects(child, subject_type, out);
            }
        }
        ExpandTree::Union { children } | ExpandTree::Intersection { children } => {
            for child in children {
                collect_subjects(child, subject_type, out);
            }
        }
        ExpandTree::Exclusion { base, .. } => {
            // Only collect from `base` — subjects in `excluded` are removed
            // by the exclusion operation, so including them would be incorrect.
            collect_subjects(base, subject_type, out);
        }
        ExpandTree::Arrow { children, .. } => {
            for child in children {
                collect_subjects(child, subject_type, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kraalzibar_core::tuple::ObjectRef;
    use kraalzibar_storage::InMemoryStoreFactory;
    use std::sync::atomic::{AtomicU64, Ordering};

    use kraalzibar_core::tuple::{
        SnapshotToken as ST, Tuple as T, TupleFilter as TF, TupleWrite as TW,
    };
    use kraalzibar_storage::traits::{RelationshipStore as RS, SchemaStore as SS, StorageError};

    /// A store wrapper that delegates to an inner store and counts read_schema/read calls.
    #[derive(Clone)]
    struct CountingStore {
        inner: kraalzibar_storage::InMemoryStore,
        read_schema_count: Arc<AtomicU64>,
        read_tuples_count: Arc<AtomicU64>,
    }

    impl RS for CountingStore {
        async fn write(&self, writes: &[TW], deletes: &[TF]) -> Result<ST, StorageError> {
            self.inner.write(writes, deletes).await
        }
        async fn read(
            &self,
            filter: &TF,
            snapshot: Option<ST>,
            limit: Option<usize>,
        ) -> Result<Vec<T>, StorageError> {
            self.read_tuples_count.fetch_add(1, Ordering::SeqCst);
            self.inner.read(filter, snapshot, limit).await
        }
        async fn snapshot(&self) -> Result<ST, StorageError> {
            self.inner.snapshot().await
        }
    }

    impl SS for CountingStore {
        async fn write_schema(&self, definition: &str) -> Result<(), StorageError> {
            self.inner.write_schema(definition).await
        }
        async fn read_schema(&self) -> Result<Option<String>, StorageError> {
            self.read_schema_count.fetch_add(1, Ordering::SeqCst);
            self.inner.read_schema().await
        }
    }

    struct CountingStoreFactory {
        inner: InMemoryStoreFactory,
        read_schema_count: Arc<AtomicU64>,
        read_tuples_count: Arc<AtomicU64>,
    }

    impl CountingStoreFactory {
        fn new() -> Self {
            Self {
                inner: InMemoryStoreFactory::new(),
                read_schema_count: Arc::new(AtomicU64::new(0)),
                read_tuples_count: Arc::new(AtomicU64::new(0)),
            }
        }

        fn read_schema_count(&self) -> u64 {
            self.read_schema_count.load(Ordering::SeqCst)
        }

        fn read_tuples_count(&self) -> u64 {
            self.read_tuples_count.load(Ordering::SeqCst)
        }
    }

    impl kraalzibar_storage::traits::StoreFactory for CountingStoreFactory {
        type Store = CountingStore;

        fn for_tenant(&self, tenant_id: &TenantId) -> CountingStore {
            CountingStore {
                inner: self.inner.for_tenant(tenant_id),
                read_schema_count: Arc::clone(&self.read_schema_count),
                read_tuples_count: Arc::clone(&self.read_tuples_count),
            }
        }
    }

    const SCHEMA: &str = r#"
        definition user {}

        definition document {
            relation owner: user
            relation editor: user
            relation viewer: user

            permission can_edit = owner + editor
            permission can_view = can_edit + viewer
        }
    "#;

    fn make_service() -> (AuthzService<InMemoryStoreFactory>, TenantId) {
        let factory = Arc::new(InMemoryStoreFactory::new());
        let service = AuthzService::new(
            Arc::clone(&factory),
            EngineConfig::default(),
            SchemaLimits::default(),
        );
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        (service, tenant_id)
    }

    async fn setup_schema(service: &AuthzService<InMemoryStoreFactory>, tenant_id: &TenantId) {
        service
            .write_schema(tenant_id, SCHEMA, false)
            .await
            .unwrap();
    }

    async fn write_tuple<F: StoreFactory>(
        service: &AuthzService<F>,
        tenant_id: &TenantId,
        object_type: &str,
        object_id: &str,
        relation: &str,
        subject_type: &str,
        subject_id: &str,
    ) -> SnapshotToken
    where
        F::Store: RelationshipStore + SchemaStore,
    {
        service
            .write_relationships(
                tenant_id,
                &[TupleWrite::new(
                    ObjectRef::new(object_type, object_id),
                    relation,
                    SubjectRef::direct(subject_type, subject_id),
                )],
                &[],
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn check_permission_grants_direct_relation() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_permission_denies_when_no_relation() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;

        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn check_permission_unknown_type_returns_error() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;

        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "nonexistent".to_string(),
                    object_id: "x".to_string(),
                    permission: "view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn write_and_read_relationships_round_trip() {
        let (service, tenant_id) = make_service();

        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let filter = TupleFilter {
            object_type: Some("document".to_string()),
            ..Default::default()
        };
        let output = service
            .read_relationships(&tenant_id, &filter, Consistency::MinimizeLatency, None)
            .await
            .unwrap();

        assert_eq!(output.tuples.len(), 1);
        assert_eq!(output.tuples[0].subject.subject_id, "alice");
        assert!(token.value() > 0);
    }

    #[tokio::test]
    async fn read_relationships_respects_consistency() {
        let (service, tenant_id) = make_service();

        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        write_tuple(
            &service, &tenant_id, "document", "readme", "editor", "user", "bob",
        )
        .await;

        let filter = TupleFilter::default();
        let output = service
            .read_relationships(
                &tenant_id,
                &filter,
                Consistency::AtExactSnapshot(token),
                None,
            )
            .await
            .unwrap();

        assert_eq!(output.tuples.len(), 1, "should only see tuples at snapshot");
    }

    #[tokio::test]
    async fn read_relationships_returns_snapshot_with_full_consistency() {
        let (service, tenant_id) = make_service();

        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let filter = TupleFilter::default();
        let output = service
            .read_relationships(&tenant_id, &filter, Consistency::FullConsistency, None)
            .await
            .unwrap();

        assert!(
            output.snapshot.is_some(),
            "FullConsistency should return a snapshot"
        );
        assert!(output.snapshot.unwrap().value() > 0);
    }

    #[tokio::test]
    async fn read_relationships_returns_no_snapshot_with_minimize_latency() {
        let (service, tenant_id) = make_service();

        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let filter = TupleFilter::default();
        let output = service
            .read_relationships(&tenant_id, &filter, Consistency::MinimizeLatency, None)
            .await
            .unwrap();

        assert!(
            output.snapshot.is_none(),
            "MinimizeLatency should not return a snapshot"
        );
    }

    #[tokio::test]
    async fn write_schema_stores_and_reads() {
        let (service, tenant_id) = make_service();

        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let schema_text = service.read_schema(&tenant_id).await.unwrap();
        assert!(schema_text.is_some());
        assert!(schema_text.unwrap().contains("document"));
    }

    #[tokio::test]
    async fn write_schema_rejects_breaking_changes_without_force() {
        let (service, tenant_id) = make_service();

        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let breaking_schema = "definition user {}";
        let result = service
            .write_schema(&tenant_id, breaking_schema, false)
            .await;

        assert!(
            matches!(result, Err(ApiError::BreakingChanges(_))),
            "expected BreakingChanges error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn write_schema_accepts_breaking_changes_with_force() {
        let (service, tenant_id) = make_service();

        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let breaking_schema = "definition user {}";
        let result = service
            .write_schema(&tenant_id, breaking_schema, true)
            .await
            .unwrap();

        assert!(result.breaking_changes_overridden);

        let stored = service.read_schema(&tenant_id).await.unwrap().unwrap();
        assert!(!stored.contains("document"));
    }

    #[tokio::test]
    async fn read_schema_returns_none_when_empty() {
        let (service, tenant_id) = make_service();

        let result = service.read_schema(&tenant_id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn expand_permission_tree_returns_tree() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "owner", "user", "bob",
        )
        .await;

        let output = service
            .expand_permission_tree(
                &tenant_id,
                ExpandPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        assert!(matches!(output.tree, ExpandTree::Union { .. }));
    }

    #[tokio::test]
    async fn lookup_subjects_returns_matching_subjects() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "owner", "user", "bob",
        )
        .await;

        let output = service
            .lookup_subjects(
                &tenant_id,
                LookupSubjectsInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        let ids: Vec<&str> = output
            .subjects
            .iter()
            .map(|s| s.subject_id.as_str())
            .collect();
        assert!(ids.contains(&"alice"), "should contain alice: {ids:?}");
        assert!(ids.contains(&"bob"), "should contain bob: {ids:?}");
    }

    #[tokio::test]
    async fn lookup_resources_returns_accessible_resources() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "design", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "secret", "viewer", "user", "bob",
        )
        .await;

        let output = service
            .lookup_resources(
                &tenant_id,
                LookupResourcesInput {
                    resource_type: "document".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                    limit: None,
                },
            )
            .await
            .unwrap();

        assert!(
            output.resource_ids.contains(&"readme".to_string()),
            "should contain readme: {:?}",
            output.resource_ids
        );
        assert!(
            output.resource_ids.contains(&"design".to_string()),
            "should contain design: {:?}",
            output.resource_ids
        );
        assert!(
            !output.resource_ids.contains(&"secret".to_string()),
            "should not contain secret: {:?}",
            output.resource_ids
        );
    }

    #[tokio::test]
    async fn lookup_resources_respects_limit() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "doc1", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "doc2", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_id, "document", "doc3", "viewer", "user", "alice",
        )
        .await;

        let output = service
            .lookup_resources(
                &tenant_id,
                LookupResourcesInput {
                    resource_type: "document".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                    limit: Some(2),
                },
            )
            .await
            .unwrap();

        assert_eq!(output.resource_ids.len(), 2, "should respect limit of 2");
    }

    #[tokio::test]
    async fn consistency_full_returns_snapshot() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::FullConsistency,
                },
            )
            .await
            .unwrap();

        assert!(result.allowed);
        assert!(
            result.snapshot.is_some(),
            "full consistency should return snapshot"
        );
    }

    #[tokio::test]
    async fn check_permission_without_schema_returns_error() {
        let (service, tenant_id) = make_service();

        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await;

        assert!(
            matches!(result, Err(ApiError::SchemaNotFound)),
            "expected SchemaNotFound, got: {result:?}"
        );
    }

    // --- 4A: Schema Cache Tests ---

    fn make_counting_service() -> (
        AuthzService<CountingStoreFactory>,
        TenantId,
        Arc<CountingStoreFactory>,
    ) {
        let factory = Arc::new(CountingStoreFactory::new());
        let service = AuthzService::new(
            Arc::clone(&factory),
            EngineConfig::default(),
            SchemaLimits::default(),
        );
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        (service, tenant_id, factory)
    }

    #[tokio::test]
    async fn schema_cache_avoids_repeated_storage_reads() {
        let (service, tenant_id, factory) = make_counting_service();

        // Write schema
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();
        assert_eq!(
            factory.read_schema_count(),
            1,
            "write_schema reads existing schema"
        );

        // First check — should read schema from store (cache miss)
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        let count_after_first = factory.read_schema_count();

        // Second check — should use cached schema (no new read_schema call)
        service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            factory.read_schema_count(),
            count_after_first,
            "second check should use cached schema"
        );
    }

    #[tokio::test]
    async fn schema_cache_invalidated_on_write_schema() {
        let (service, tenant_id, _factory) = make_counting_service();

        // Write initial schema (no permission can_edit)
        let schema_v1 = r#"
            definition user {}
            definition document {
                relation viewer: user
                permission can_view = viewer
            }
        "#;
        service
            .write_schema(&tenant_id, schema_v1, false)
            .await
            .unwrap();

        // Warm cache by checking
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        // Write new schema with can_edit permission (force: breaking change removes can_view)
        let schema_v2 = r#"
            definition user {}
            definition document {
                relation viewer: user
                relation editor: user
                permission can_view = viewer + editor
                permission can_edit = editor
            }
        "#;
        service
            .write_schema(&tenant_id, schema_v2, true)
            .await
            .unwrap();

        // Now check can_edit — this should work with the new schema
        write_tuple(
            &service, &tenant_id, "document", "readme", "editor", "user", "bob",
        )
        .await;
        let result = service
            .check_permission(
                &tenant_id,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_edit".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "bob".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();

        assert!(
            result.allowed,
            "should use new schema with can_edit permission"
        );
    }

    #[tokio::test]
    async fn schema_cache_isolated_per_tenant() {
        let (service, tenant_a, _factory) = make_counting_service();
        let tenant_b = TenantId::new(uuid::Uuid::new_v4());

        let schema_a = r#"
            definition user {}
            definition document {
                relation viewer: user
                permission can_view = viewer
            }
        "#;
        let schema_b = r#"
            definition user {}
            definition folder {
                relation reader: user
                permission can_read = reader
            }
        "#;

        service
            .write_schema(&tenant_a, schema_a, false)
            .await
            .unwrap();
        service
            .write_schema(&tenant_b, schema_b, false)
            .await
            .unwrap();

        write_tuple(
            &service, &tenant_a, "document", "readme", "viewer", "user", "alice",
        )
        .await;
        write_tuple(
            &service, &tenant_b, "folder", "root", "reader", "user", "bob",
        )
        .await;

        // Tenant A check should use document schema
        let result_a = service
            .check_permission(
                &tenant_a,
                CheckPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();
        assert!(result_a.allowed);

        // Tenant B check should use folder schema
        let result_b = service
            .check_permission(
                &tenant_b,
                CheckPermissionInput {
                    object_type: "folder".to_string(),
                    object_id: "root".to_string(),
                    permission: "can_read".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "bob".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await
            .unwrap();
        assert!(result_b.allowed);

        // Cross-check: tenant A should NOT have folder type
        let result_cross = service
            .check_permission(
                &tenant_a,
                CheckPermissionInput {
                    object_type: "folder".to_string(),
                    object_id: "root".to_string(),
                    permission: "can_read".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "bob".to_string(),
                    consistency: Consistency::MinimizeLatency,
                },
            )
            .await;
        assert!(
            result_cross.is_err(),
            "tenant A should not have folder schema"
        );
    }

    // --- 4B: Check Result Cache Tests ---

    fn make_check_input(
        object_type: &str,
        object_id: &str,
        permission: &str,
        subject_type: &str,
        subject_id: &str,
        consistency: Consistency,
    ) -> CheckPermissionInput {
        CheckPermissionInput {
            object_type: object_type.to_string(),
            object_id: object_id.to_string(),
            permission: permission.to_string(),
            subject_type: subject_type.to_string(),
            subject_id: subject_id.to_string(),
            consistency,
        }
    }

    #[tokio::test]
    async fn check_cache_returns_cached_result() {
        let (service, tenant_id, factory) = make_counting_service();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();
        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // First check with explicit snapshot — runs engine
        let result1 = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();
        assert!(result1.allowed);

        let reads_after_first = factory.read_tuples_count();

        // Second check with same snapshot — should return cached, no new tuple reads
        let result2 = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();
        assert!(result2.allowed);

        assert_eq!(
            factory.read_tuples_count(),
            reads_after_first,
            "second check should use cached result without reading tuples"
        );
    }

    #[tokio::test]
    async fn check_cache_skips_without_snapshot() {
        let (service, tenant_id, factory) = make_counting_service();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // MinimizeLatency — snapshot is None, should NOT cache
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::MinimizeLatency,
                ),
            )
            .await
            .unwrap();

        let reads_after_first = factory.read_tuples_count();

        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::MinimizeLatency,
                ),
            )
            .await
            .unwrap();

        assert!(
            factory.read_tuples_count() > reads_after_first,
            "MinimizeLatency should not use check cache"
        );
    }

    #[tokio::test]
    async fn check_cache_invalidated_on_write_relationships() {
        let (service, tenant_id, _factory) = make_counting_service();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // Populate cache
        let result = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();
        assert!(result.allowed);

        // Write new relationship — should invalidate check cache
        write_tuple(
            &service, &tenant_id, "document", "readme", "editor", "user", "bob",
        )
        .await;

        // Check with the OLD token — must still work correctly
        // (cache was invalidated, so it re-runs the engine)
        let result2 = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();
        assert!(result2.allowed);
    }

    #[tokio::test]
    async fn check_cache_invalidated_on_write_schema() {
        let (service, tenant_id, _factory) = make_counting_service();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // Populate check cache
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();

        // Write schema — should invalidate check cache
        service
            .write_schema(&tenant_id, SCHEMA, true)
            .await
            .unwrap();

        // The check should still work (cache invalidated, re-runs engine)
        let result = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_cache_isolated_per_tenant() {
        let (service, tenant_id_a, factory) = make_counting_service();
        let tenant_id_b = TenantId::new(uuid::Uuid::new_v4());

        service
            .write_schema(&tenant_id_a, SCHEMA, false)
            .await
            .unwrap();
        service
            .write_schema(&tenant_id_b, SCHEMA, false)
            .await
            .unwrap();

        let token_a = write_tuple(
            &service,
            &tenant_id_a,
            "document",
            "readme",
            "viewer",
            "user",
            "alice",
        )
        .await;

        // Tenant A check — allowed
        let result_a = service
            .check_permission(
                &tenant_id_a,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token_a),
                ),
            )
            .await
            .unwrap();
        assert!(result_a.allowed);

        // Get a snapshot for tenant B (which has no tuples)
        let store_b = factory.inner.for_tenant(&tenant_id_b);
        let token_b = store_b.snapshot().await.unwrap();

        // Tenant B check for same params — should be denied (no tuples)
        let result_b = service
            .check_permission(
                &tenant_id_b,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token_b),
                ),
            )
            .await
            .unwrap();
        assert!(!result_b.allowed, "tenant B should not see tenant A tuples");
    }

    #[tokio::test]
    async fn check_cache_different_snapshots_are_separate() {
        let (service, tenant_id, _factory) = make_counting_service();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        // snapshot 1: no viewer tuple, so check should be denied
        let store = _factory.inner.for_tenant(&tenant_id);
        let snap1 = store.snapshot().await.unwrap();
        let result1 = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(snap1),
                ),
            )
            .await
            .unwrap();
        assert!(!result1.allowed);

        // Write tuple, get snapshot 2
        let snap2 = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // snapshot 2: viewer tuple exists, check should be allowed
        let result2 = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(snap2),
                ),
            )
            .await
            .unwrap();
        assert!(result2.allowed);

        // Re-check at snapshot 1 — should still be denied (cached separately)
        let result1_again = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(snap1),
                ),
            )
            .await
            .unwrap();
        assert!(
            !result1_again.allowed,
            "snapshot 1 result should be cached as denied"
        );
    }

    // --- 4C: Snapshot Token Tests ---

    #[tokio::test]
    async fn expand_response_includes_snapshot_token() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let output = service
            .expand_permission_tree(
                &tenant_id,
                ExpandPermissionInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    consistency: Consistency::FullConsistency,
                },
            )
            .await
            .unwrap();

        assert!(
            output.snapshot.is_some(),
            "FullConsistency expand should return snapshot token"
        );
    }

    #[tokio::test]
    async fn lookup_resources_response_includes_snapshot_token() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let output = service
            .lookup_resources(
                &tenant_id,
                LookupResourcesInput {
                    resource_type: "document".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    consistency: Consistency::FullConsistency,
                    limit: None,
                },
            )
            .await
            .unwrap();

        assert!(
            output.snapshot.is_some(),
            "FullConsistency lookup_resources should return snapshot token"
        );
        assert!(output.resource_ids.contains(&"readme".to_string()));
    }

    #[tokio::test]
    async fn lookup_subjects_response_includes_snapshot_token() {
        let (service, tenant_id) = make_service();
        setup_schema(&service, &tenant_id).await;
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        let output = service
            .lookup_subjects(
                &tenant_id,
                LookupSubjectsInput {
                    object_type: "document".to_string(),
                    object_id: "readme".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    consistency: Consistency::FullConsistency,
                },
            )
            .await
            .unwrap();

        assert!(
            output.snapshot.is_some(),
            "FullConsistency lookup_subjects should return snapshot token"
        );
        assert!(!output.subjects.is_empty());
    }

    // --- 4F: Cache Metrics Tests ---

    fn make_counting_service_with_metrics() -> (
        AuthzService<CountingStoreFactory>,
        TenantId,
        Arc<CountingStoreFactory>,
        Arc<Metrics>,
    ) {
        let factory = Arc::new(CountingStoreFactory::new());
        let metrics = Arc::new(Metrics::new());
        let service = AuthzService::new(
            Arc::clone(&factory),
            EngineConfig::default(),
            SchemaLimits::default(),
        )
        .with_metrics(Arc::clone(&metrics));
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        (service, tenant_id, factory, metrics)
    }

    #[tokio::test]
    async fn schema_cache_hit_increments_metric() {
        let (service, tenant_id, _factory, metrics) = make_counting_service_with_metrics();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        // First check_permission loads schema (cache miss)
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::MinimizeLatency,
                ),
            )
            .await
            .unwrap();

        assert_eq!(metrics.schema_cache_misses(), 1);
        assert_eq!(metrics.schema_cache_hits(), 0);

        // Second call hits schema cache
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::MinimizeLatency,
                ),
            )
            .await
            .unwrap();

        assert_eq!(metrics.schema_cache_hits(), 1);
    }

    #[tokio::test]
    async fn check_cache_hit_increments_metric() {
        let (service, tenant_id, _factory, metrics) = make_counting_service_with_metrics();
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        let token = write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        // First check with snapshot (cache miss)
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();

        assert_eq!(metrics.check_cache_misses(), 1);
        assert_eq!(metrics.check_cache_hits(), 0);

        // Second check with same snapshot (cache hit)
        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();

        assert_eq!(metrics.check_cache_hits(), 1);
    }

    // --- 6B: Audit Logging Tests ---

    mod audit_test_helpers {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Debug)]
        pub struct CapturedEvent {
            pub target: String,
            pub fields: Vec<(String, String)>,
        }

        struct TestLayer {
            events: Arc<Mutex<Vec<CapturedEvent>>>,
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TestLayer {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut fields = Vec::new();
                let mut visitor = FieldVisitor(&mut fields);
                event.record(&mut visitor);
                self.events.lock().unwrap().push(CapturedEvent {
                    target: event.metadata().target().to_string(),
                    fields,
                });
            }
        }

        struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

        impl tracing::field::Visit for FieldVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0
                    .push((field.name().to_string(), format!("{value:?}")));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.push((field.name().to_string(), value.to_string()));
            }
            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                self.0.push((field.name().to_string(), value.to_string()));
            }
            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                self.0.push((field.name().to_string(), value.to_string()));
            }
        }

        pub fn make_subscriber() -> (
            impl tracing::Subscriber + Send + Sync,
            Arc<Mutex<Vec<CapturedEvent>>>,
        ) {
            let events = Arc::new(Mutex::new(Vec::new()));
            let layer = TestLayer {
                events: Arc::clone(&events),
            };
            let subscriber = tracing_subscriber::registry().with(layer);
            (subscriber, events)
        }

        pub fn has_field(event: &CapturedEvent, key: &str, value: &str) -> bool {
            event.fields.iter().any(|(k, v)| k == key && v == value)
        }
    }

    #[tokio::test]
    async fn service_write_schema_emits_audit_event() {
        let (service, tenant_id) = make_service();
        let (subscriber, events) = audit_test_helpers::make_subscriber();

        let _guard = tracing::subscriber::set_default(subscriber);

        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        drop(_guard);

        let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
        let audit_events: Vec<_> = events.iter().filter(|e| e.target == "audit").collect();
        assert!(
            !audit_events.is_empty(),
            "write_schema should emit audit event"
        );
        assert!(audit_test_helpers::has_field(
            audit_events[0],
            "event",
            "schema_write"
        ));
    }

    #[tokio::test]
    async fn service_write_relationships_emits_audit_event() {
        let (service, tenant_id) = make_service();
        let (subscriber, events) = audit_test_helpers::make_subscriber();

        let _guard = tracing::subscriber::set_default(subscriber);

        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        drop(_guard);

        let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();
        let audit_events: Vec<_> = events.iter().filter(|e| e.target == "audit").collect();
        assert!(
            !audit_events.is_empty(),
            "write_relationships should emit audit event"
        );
        assert!(audit_test_helpers::has_field(
            audit_events[0],
            "event",
            "relationship_write"
        ));
    }

    // --- 6C: Tracing Span Tests ---

    mod span_test_helpers {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Debug)]
        pub struct CapturedSpan {
            pub name: String,
            pub fields: Vec<(String, String)>,
        }

        struct SpanLayer {
            spans: Arc<Mutex<Vec<CapturedSpan>>>,
        }

        impl<S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>>
            tracing_subscriber::Layer<S> for SpanLayer
        {
            fn on_new_span(
                &self,
                attrs: &tracing::span::Attributes<'_>,
                _id: &tracing::span::Id,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut fields = Vec::new();
                let mut visitor = FieldVisitor(&mut fields);
                attrs.record(&mut visitor);
                self.spans.lock().unwrap().push(CapturedSpan {
                    name: attrs.metadata().name().to_string(),
                    fields,
                });
            }
        }

        struct FieldVisitor<'a>(&'a mut Vec<(String, String)>);

        impl tracing::field::Visit for FieldVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0
                    .push((field.name().to_string(), format!("{value:?}")));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.push((field.name().to_string(), value.to_string()));
            }
        }

        pub fn make_span_subscriber() -> (
            impl tracing::Subscriber + Send + Sync,
            Arc<Mutex<Vec<CapturedSpan>>>,
        ) {
            let spans = Arc::new(Mutex::new(Vec::new()));
            let layer = SpanLayer {
                spans: Arc::clone(&spans),
            };
            let subscriber = tracing_subscriber::registry().with(layer);
            (subscriber, spans)
        }
    }

    #[tokio::test]
    async fn service_methods_have_tracing_spans() {
        let (service, tenant_id) = make_service();
        let (subscriber, spans) = span_test_helpers::make_span_subscriber();

        let _guard = tracing::subscriber::set_default(subscriber);

        // write_schema creates a span
        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        // check_permission creates a span
        write_tuple(
            &service, &tenant_id, "document", "readme", "viewer", "user", "alice",
        )
        .await;

        service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    "readme",
                    "can_view",
                    "user",
                    "alice",
                    Consistency::MinimizeLatency,
                ),
            )
            .await
            .unwrap();

        drop(_guard);

        let spans = Arc::try_unwrap(spans).unwrap().into_inner().unwrap();
        let span_names: Vec<&str> = spans.iter().map(|s| s.name.as_str()).collect();

        assert!(
            span_names.contains(&"check_permission"),
            "should have check_permission span: {span_names:?}"
        );
        assert!(
            span_names.contains(&"write_schema"),
            "should have write_schema span: {span_names:?}"
        );
        assert!(
            span_names.contains(&"write_relationships"),
            "should have write_relationships span: {span_names:?}"
        );
    }

    #[tokio::test]
    async fn tracing_spans_include_tenant_id() {
        let (service, tenant_id) = make_service();
        let (subscriber, spans) = span_test_helpers::make_span_subscriber();

        let _guard = tracing::subscriber::set_default(subscriber);

        service
            .write_schema(&tenant_id, SCHEMA, false)
            .await
            .unwrap();

        drop(_guard);

        let spans = Arc::try_unwrap(spans).unwrap().into_inner().unwrap();
        let write_schema_span = spans.iter().find(|s| s.name == "write_schema");
        assert!(
            write_schema_span.is_some(),
            "write_schema span should exist"
        );

        let has_tenant_id = write_schema_span
            .unwrap()
            .fields
            .iter()
            .any(|(k, _)| k == "tenant_id");
        assert!(
            has_tenant_id,
            "write_schema span should include tenant_id field"
        );
    }
}
