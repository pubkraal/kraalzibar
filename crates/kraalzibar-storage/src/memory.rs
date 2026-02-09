use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kraalzibar_core::tuple::{
    ObjectRef, SnapshotToken, SubjectRef, TenantId, Tuple, TupleFilter, TupleWrite,
};

use crate::traits::{RelationshipStore, SchemaStore, StorageError, StoreFactory};

const ACTIVE_TX_ID: u64 = u64::MAX;

#[derive(Debug, Clone)]
struct StoredTuple {
    object_type: String,
    object_id: String,
    relation: String,
    subject_type: String,
    subject_id: String,
    subject_relation: Option<String>,
    created_tx_id: u64,
    deleted_tx_id: u64,
}

impl StoredTuple {
    fn matches_write(&self, write: &TupleWrite) -> bool {
        self.object_type == write.object.object_type
            && self.object_id == write.object.object_id
            && self.relation == write.relation
            && self.subject_type == write.subject.subject_type
            && self.subject_id == write.subject.subject_id
            && self.subject_relation == write.subject.subject_relation
    }

    fn is_active(&self) -> bool {
        self.deleted_tx_id == ACTIVE_TX_ID
    }

    fn visible_at(&self, snapshot: u64) -> bool {
        self.created_tx_id <= snapshot && self.deleted_tx_id > snapshot
    }

    fn to_tuple(&self) -> Tuple {
        let subject = match &self.subject_relation {
            None => SubjectRef::direct(&self.subject_type, &self.subject_id),
            Some(rel) => SubjectRef::userset(&self.subject_type, &self.subject_id, rel),
        };
        Tuple::new(
            ObjectRef::new(&self.object_type, &self.object_id),
            &self.relation,
            subject,
        )
    }

    fn matches_filter(&self, filter: &TupleFilter) -> bool {
        filter.matches(&self.to_tuple())
    }
}

#[derive(Debug)]
struct InnerState {
    current_tx: u64,
    tuples: Vec<StoredTuple>,
    schema: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InMemoryStore {
    state: Arc<Mutex<InnerState>>,
}

impl InMemoryStore {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(InnerState {
                current_tx: 0,
                tuples: Vec::new(),
                schema: None,
            })),
        }
    }
}

impl RelationshipStore for InMemoryStore {
    async fn write(
        &self,
        writes: &[TupleWrite],
        deletes: &[TupleFilter],
    ) -> Result<SnapshotToken, StorageError> {
        let mut state = self.state.lock().unwrap();

        for (i, w) in writes.iter().enumerate() {
            for other in &writes[i + 1..] {
                if w == other {
                    return Err(StorageError::DuplicateTuple);
                }
            }
        }

        state.current_tx += 1;
        let tx_id = state.current_tx;

        for filter in deletes {
            for tuple in &mut state.tuples {
                if tuple.is_active() && tuple.matches_filter(filter) {
                    tuple.deleted_tx_id = tx_id;
                }
            }
        }

        for w in writes {
            let active_dup = state
                .tuples
                .iter()
                .any(|t| t.is_active() && t.matches_write(w));
            if active_dup {
                return Err(StorageError::DuplicateTuple);
            }

            state.tuples.push(StoredTuple {
                object_type: w.object.object_type.clone(),
                object_id: w.object.object_id.clone(),
                relation: w.relation.clone(),
                subject_type: w.subject.subject_type.clone(),
                subject_id: w.subject.subject_id.clone(),
                subject_relation: w.subject.subject_relation.clone(),
                created_tx_id: tx_id,
                deleted_tx_id: ACTIVE_TX_ID,
            });
        }

        Ok(SnapshotToken::new(tx_id))
    }

    async fn read(
        &self,
        filter: &TupleFilter,
        snapshot: Option<SnapshotToken>,
    ) -> Result<Vec<Tuple>, StorageError> {
        let state = self.state.lock().unwrap();

        let snap = match snapshot {
            Some(token) => {
                let val = token.value();
                if val > state.current_tx {
                    return Err(StorageError::SnapshotAhead {
                        requested: val,
                        current: state.current_tx,
                    });
                }
                val
            }
            None => state.current_tx,
        };

        let results = state
            .tuples
            .iter()
            .filter(|t| t.visible_at(snap) && t.matches_filter(filter))
            .map(|t| t.to_tuple())
            .collect();

        Ok(results)
    }

    async fn snapshot(&self) -> Result<SnapshotToken, StorageError> {
        let state = self.state.lock().unwrap();
        Ok(SnapshotToken::new(state.current_tx))
    }
}

impl SchemaStore for InMemoryStore {
    async fn write_schema(&self, definition: &str) -> Result<(), StorageError> {
        let mut state = self.state.lock().unwrap();
        state.schema = Some(definition.to_string());
        Ok(())
    }

    async fn read_schema(&self) -> Result<Option<String>, StorageError> {
        let state = self.state.lock().unwrap();
        Ok(state.schema.clone())
    }
}

#[derive(Debug)]
pub struct InMemoryStoreFactory {
    stores: Mutex<HashMap<TenantId, InMemoryStore>>,
}

impl InMemoryStoreFactory {
    pub fn new() -> Self {
        Self {
            stores: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStoreFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreFactory for InMemoryStoreFactory {
    type Store = InMemoryStore;

    fn for_tenant(&self, tenant_id: &TenantId) -> InMemoryStore {
        let mut stores = self.stores.lock().unwrap();
        stores
            .entry(tenant_id.clone())
            .or_insert_with(InMemoryStore::new)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_write(obj_type: &str, obj_id: &str, relation: &str, subj: SubjectRef) -> TupleWrite {
        TupleWrite::new(ObjectRef::new(obj_type, obj_id), relation, subj)
    }

    fn direct_user(name: &str) -> SubjectRef {
        SubjectRef::direct("user", name)
    }

    fn userset_group(group: &str, rel: &str) -> SubjectRef {
        SubjectRef::userset("group", group, rel)
    }

    // 1. Fresh store snapshot is 0
    #[tokio::test]
    async fn fresh_store_snapshot_is_zero() {
        let store = InMemoryStore::new();

        let token = store.snapshot().await.unwrap();

        assert_eq!(token.value(), 0);
    }

    // 2. Write returns incrementing tokens
    #[tokio::test]
    async fn write_returns_incrementing_tokens() {
        let store = InMemoryStore::new();

        let t1 = store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();
        let t2 = store
            .write(&[make_write("doc", "2", "viewer", direct_user("b"))], &[])
            .await
            .unwrap();

        assert_eq!(t1.value(), 1);
        assert_eq!(t2.value(), 2);
    }

    // 3. Written tuple can be read back
    #[tokio::test]
    async fn written_tuple_can_be_read_back() {
        let store = InMemoryStore::new();
        store
            .write(
                &[make_write("doc", "readme", "viewer", direct_user("john"))],
                &[],
            )
            .await
            .unwrap();

        let results = store.read(&TupleFilter::default(), None).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object, ObjectRef::new("doc", "readme"));
        assert_eq!(results[0].relation, "viewer");
        assert_eq!(results[0].subject, SubjectRef::direct("user", "john"));
    }

    // 4. Empty filter returns all active tuples
    #[tokio::test]
    async fn empty_filter_returns_all_active_tuples() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", direct_user("a")),
                    make_write("doc", "2", "editor", direct_user("b")),
                ],
                &[],
            )
            .await
            .unwrap();

        let results = store.read(&TupleFilter::default(), None).await.unwrap();

        assert_eq!(results.len(), 2);
    }

    // 5. Specific filter matches only matching tuples
    #[tokio::test]
    async fn specific_filter_matches_only_matching_tuples() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", direct_user("a")),
                    make_write("doc", "2", "editor", direct_user("b")),
                ],
                &[],
            )
            .await
            .unwrap();

        let filter = TupleFilter {
            relation: Some("editor".to_string()),
            ..Default::default()
        };
        let results = store.read(&filter, None).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].relation, "editor");
    }

    // 6. Multiple writes increment tokens
    #[tokio::test]
    async fn multiple_writes_increment_tokens() {
        let store = InMemoryStore::new();

        let t1 = store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();
        let t2 = store
            .write(&[make_write("doc", "2", "viewer", direct_user("b"))], &[])
            .await
            .unwrap();
        let t3 = store
            .write(&[make_write("doc", "3", "viewer", direct_user("c"))], &[])
            .await
            .unwrap();

        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    // 7. Snapshot-consistent read sees only tuples at/before snapshot
    #[tokio::test]
    async fn snapshot_consistent_read() {
        let store = InMemoryStore::new();

        let snap1 = store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();
        store
            .write(&[make_write("doc", "2", "viewer", direct_user("b"))], &[])
            .await
            .unwrap();

        let results = store
            .read(&TupleFilter::default(), Some(snap1))
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object.object_id, "1");
    }

    // 8. Deleted tuples invisible after deletion
    #[tokio::test]
    async fn deleted_tuples_invisible_after_deletion() {
        let store = InMemoryStore::new();
        store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();

        let delete_filter = TupleFilter {
            object_type: Some("doc".to_string()),
            object_id: Some("1".to_string()),
            ..Default::default()
        };
        store.write(&[], &[delete_filter]).await.unwrap();

        let results = store.read(&TupleFilter::default(), None).await.unwrap();

        assert!(results.is_empty());
    }

    // 9. Deleted tuples visible at pre-deletion snapshot
    #[tokio::test]
    async fn deleted_tuples_visible_at_pre_deletion_snapshot() {
        let store = InMemoryStore::new();
        let snap = store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();

        let delete_filter = TupleFilter {
            object_type: Some("doc".to_string()),
            object_id: Some("1".to_string()),
            ..Default::default()
        };
        store.write(&[], &[delete_filter]).await.unwrap();

        let results = store
            .read(&TupleFilter::default(), Some(snap))
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
    }

    // 10. Duplicate tuple in same write rejected
    #[tokio::test]
    async fn duplicate_tuple_in_same_write_rejected() {
        let store = InMemoryStore::new();
        let w = make_write("doc", "1", "viewer", direct_user("a"));

        let result = store.write(&[w.clone(), w], &[]).await;

        assert!(matches!(result, Err(StorageError::DuplicateTuple)));
    }

    // 11. Re-write previously deleted tuple creates new version
    #[tokio::test]
    async fn rewrite_deleted_tuple_creates_new_version() {
        let store = InMemoryStore::new();
        let w = make_write("doc", "1", "viewer", direct_user("a"));
        store.write(&[w.clone()], &[]).await.unwrap();

        let delete_filter = TupleFilter {
            object_type: Some("doc".to_string()),
            object_id: Some("1".to_string()),
            ..Default::default()
        };
        store.write(&[], &[delete_filter]).await.unwrap();

        store.write(&[w], &[]).await.unwrap();

        let results = store.read(&TupleFilter::default(), None).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    // 12. Read rejects snapshot ahead of current
    #[tokio::test]
    async fn read_rejects_snapshot_ahead_of_current() {
        let store = InMemoryStore::new();

        let result = store
            .read(&TupleFilter::default(), Some(SnapshotToken::new(999)))
            .await;

        assert!(matches!(result, Err(StorageError::SnapshotAhead { .. })));
    }

    // 13. Write with both writes and deletes is atomic (same tx_id)
    #[tokio::test]
    async fn write_and_delete_atomic_same_tx() {
        let store = InMemoryStore::new();
        let snap_before = store
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();

        let delete_filter = TupleFilter {
            object_type: Some("doc".to_string()),
            object_id: Some("1".to_string()),
            ..Default::default()
        };
        let snap_after = store
            .write(
                &[make_write("doc", "2", "viewer", direct_user("b"))],
                &[delete_filter],
            )
            .await
            .unwrap();

        let at_before = store
            .read(&TupleFilter::default(), Some(snap_before))
            .await
            .unwrap();
        assert_eq!(at_before.len(), 1);
        assert_eq!(at_before[0].object.object_id, "1");

        let at_after = store
            .read(&TupleFilter::default(), Some(snap_after))
            .await
            .unwrap();
        assert_eq!(at_after.len(), 1);
        assert_eq!(at_after[0].object.object_id, "2");
    }

    // 14. Filter by subject_relation Some(None) matches only direct
    #[tokio::test]
    async fn filter_subject_relation_none_matches_direct() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", direct_user("a")),
                    make_write("doc", "1", "viewer", userset_group("eng", "member")),
                ],
                &[],
            )
            .await
            .unwrap();

        let filter = TupleFilter {
            subject_relation: Some(None),
            ..Default::default()
        };
        let results = store.read(&filter, None).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].subject.subject_relation, None);
    }

    // 15. Filter by subject_relation Some(Some("member")) matches usersets
    #[tokio::test]
    async fn filter_subject_relation_some_matches_usersets() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", direct_user("a")),
                    make_write("doc", "1", "viewer", userset_group("eng", "member")),
                ],
                &[],
            )
            .await
            .unwrap();

        let filter = TupleFilter {
            subject_relation: Some(Some("member".to_string())),
            ..Default::default()
        };
        let results = store.read(&filter, None).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].subject.subject_relation,
            Some("member".to_string())
        );
    }

    // 16. Factory returns same store for same tenant
    #[tokio::test]
    async fn factory_returns_same_store_for_same_tenant() {
        let factory = InMemoryStoreFactory::new();
        let tenant = TenantId::new(Uuid::new_v4());

        let store1 = factory.for_tenant(&tenant);
        store1
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();

        let store2 = factory.for_tenant(&tenant);
        let results = store2.read(&TupleFilter::default(), None).await.unwrap();

        assert_eq!(results.len(), 1);
    }

    // 17. Factory returns different stores for different tenants
    #[tokio::test]
    async fn factory_returns_different_stores_for_different_tenants() {
        let factory = InMemoryStoreFactory::new();
        let tenant_a = TenantId::new(Uuid::new_v4());
        let tenant_b = TenantId::new(Uuid::new_v4());

        let store_a = factory.for_tenant(&tenant_a);
        let store_b = factory.for_tenant(&tenant_b);

        store_a
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();

        let results_b = store_b.read(&TupleFilter::default(), None).await.unwrap();

        assert!(results_b.is_empty());
    }

    // 18. Stores are isolated across tenants
    #[tokio::test]
    async fn stores_isolated_across_tenants() {
        let factory = InMemoryStoreFactory::new();
        let tenant_a = TenantId::new(Uuid::new_v4());
        let tenant_b = TenantId::new(Uuid::new_v4());

        let store_a = factory.for_tenant(&tenant_a);
        let store_b = factory.for_tenant(&tenant_b);

        store_a
            .write(&[make_write("doc", "1", "viewer", direct_user("a"))], &[])
            .await
            .unwrap();
        store_b
            .write(
                &[
                    make_write("doc", "2", "editor", direct_user("b")),
                    make_write("doc", "3", "editor", direct_user("c")),
                ],
                &[],
            )
            .await
            .unwrap();

        let results_a = store_a.read(&TupleFilter::default(), None).await.unwrap();
        let results_b = store_b.read(&TupleFilter::default(), None).await.unwrap();

        assert_eq!(results_a.len(), 1);
        assert_eq!(results_b.len(), 2);
    }

    // 19. Fresh store has no schema
    #[tokio::test]
    async fn fresh_store_has_no_schema() {
        let store = InMemoryStore::new();

        let schema = store.read_schema().await.unwrap();

        assert_eq!(schema, None);
    }

    // 20. write_schema stores definition text
    #[tokio::test]
    async fn write_schema_stores_definition() {
        let store = InMemoryStore::new();
        let definition = "definition user {}";

        store.write_schema(definition).await.unwrap();

        let schema = store.read_schema().await.unwrap();
        assert_eq!(schema, Some(definition.to_string()));
    }

    // 21. write_schema overwrites previous
    #[tokio::test]
    async fn write_schema_overwrites_previous() {
        let store = InMemoryStore::new();
        store.write_schema("definition user {}").await.unwrap();
        store
            .write_schema("definition group { relation member: user }")
            .await
            .unwrap();

        let schema = store.read_schema().await.unwrap();
        assert_eq!(
            schema,
            Some("definition group { relation member: user }".to_string())
        );
    }
}
