use std::sync::Arc;

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
use crate::error::ApiError;

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
pub struct WriteSchemaOutput {
    pub breaking_changes_overridden: bool,
}

pub struct AuthzService<F: StoreFactory> {
    factory: Arc<F>,
    engine_config: EngineConfig,
    schema_limits: SchemaLimits,
}

impl<F: StoreFactory> AuthzService<F>
where
    F::Store: RelationshipStore + SchemaStore,
{
    pub fn new(factory: Arc<F>, engine_config: EngineConfig, schema_limits: SchemaLimits) -> Self {
        Self {
            factory,
            engine_config,
            schema_limits,
        }
    }

    pub async fn check_permission(
        &self,
        tenant_id: &TenantId,
        input: CheckPermissionInput,
    ) -> Result<CheckPermissionOutput, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let store = Arc::new(store);

        let snapshot = self.resolve_snapshot(&*store, input.consistency).await?;
        let schema = self.load_schema(&*store).await?;

        let reader = StoreTupleReader::new(Arc::clone(&store));
        let engine = CheckEngine::new(
            Arc::new(reader),
            Arc::new(schema),
            self.engine_config.clone(),
        );

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

    pub async fn expand_permission_tree(
        &self,
        tenant_id: &TenantId,
        input: ExpandPermissionInput,
    ) -> Result<ExpandTree, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let store = Arc::new(store);

        let snapshot = self.resolve_snapshot(&*store, input.consistency).await?;
        let schema = self.load_schema(&*store).await?;

        let reader = StoreTupleReader::new(Arc::clone(&store));
        let engine = ExpandEngine::new(
            Arc::new(reader),
            Arc::new(schema),
            self.engine_config.clone(),
        );

        let request = ExpandRequest {
            object_type: input.object_type,
            object_id: input.object_id,
            permission: input.permission,
            snapshot,
        };

        Ok(engine.expand(&request).await?)
    }

    pub async fn write_relationships(
        &self,
        tenant_id: &TenantId,
        writes: &[TupleWrite],
        deletes: &[TupleFilter],
    ) -> Result<SnapshotToken, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        Ok(store.write(writes, deletes).await?)
    }

    pub async fn read_relationships(
        &self,
        tenant_id: &TenantId,
        filter: &TupleFilter,
        consistency: Consistency,
        limit: Option<usize>,
    ) -> Result<Vec<Tuple>, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        let snapshot = self.resolve_snapshot(&store, consistency).await?;
        Ok(store.read(filter, snapshot, limit).await?)
    }

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

        Ok(WriteSchemaOutput {
            breaking_changes_overridden,
        })
    }

    pub async fn read_schema(&self, tenant_id: &TenantId) -> Result<Option<String>, ApiError> {
        let store = self.factory.for_tenant(tenant_id);
        Ok(store.read_schema().await?)
    }

    pub async fn lookup_subjects(
        &self,
        tenant_id: &TenantId,
        input: LookupSubjectsInput,
    ) -> Result<Vec<SubjectRef>, ApiError> {
        let tree = self
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
        collect_subjects(&tree, &input.subject_type, &mut subjects);
        subjects.dedup();
        Ok(subjects)
    }

    pub async fn lookup_resources(
        &self,
        tenant_id: &TenantId,
        input: LookupResourcesInput,
    ) -> Result<Vec<String>, ApiError> {
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

        let schema = self.load_schema(&*store).await?;

        let reader = StoreTupleReader::new(Arc::clone(&store));
        let engine = CheckEngine::new(
            Arc::new(reader),
            Arc::new(schema),
            self.engine_config.clone(),
        );

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

        Ok(results)
    }

    async fn load_schema<S: SchemaStore>(&self, store: &S) -> Result<Schema, ApiError> {
        let schema_text = store.read_schema().await?.ok_or(ApiError::SchemaNotFound)?;
        Ok(parse_schema(&schema_text)?)
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

    async fn write_tuple(
        service: &AuthzService<InMemoryStoreFactory>,
        tenant_id: &TenantId,
        object_type: &str,
        object_id: &str,
        relation: &str,
        subject_type: &str,
        subject_id: &str,
    ) -> SnapshotToken {
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
        let tuples = service
            .read_relationships(&tenant_id, &filter, Consistency::MinimizeLatency, None)
            .await
            .unwrap();

        assert_eq!(tuples.len(), 1);
        assert_eq!(tuples[0].subject.subject_id, "alice");
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
        let tuples = service
            .read_relationships(
                &tenant_id,
                &filter,
                Consistency::AtExactSnapshot(token),
                None,
            )
            .await
            .unwrap();

        assert_eq!(tuples.len(), 1, "should only see tuples at snapshot");
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

        let tree = service
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

        assert!(matches!(tree, ExpandTree::Union { .. }));
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

        let subjects = service
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

        let ids: Vec<&str> = subjects.iter().map(|s| s.subject_id.as_str()).collect();
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

        let resources = service
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
            resources.contains(&"readme".to_string()),
            "should contain readme: {resources:?}"
        );
        assert!(
            resources.contains(&"design".to_string()),
            "should contain design: {resources:?}"
        );
        assert!(
            !resources.contains(&"secret".to_string()),
            "should not contain secret: {resources:?}"
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

        let resources = service
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

        assert_eq!(resources.len(), 2, "should respect limit of 2");
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
}
