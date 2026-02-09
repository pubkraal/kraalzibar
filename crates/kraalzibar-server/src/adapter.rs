use std::sync::Arc;

use kraalzibar_core::engine::{CheckError, TupleReader};
use kraalzibar_core::tuple::{SnapshotToken, Tuple, TupleFilter};
use kraalzibar_storage::RelationshipStore;

pub struct StoreTupleReader<S: RelationshipStore> {
    store: Arc<S>,
}

impl<S: RelationshipStore> StoreTupleReader<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

impl<S: RelationshipStore> TupleReader for StoreTupleReader<S> {
    async fn read_tuples(
        &self,
        filter: &TupleFilter,
        snapshot: Option<SnapshotToken>,
    ) -> Result<Vec<Tuple>, CheckError> {
        self.store
            .read(filter, snapshot, None)
            .await
            .map_err(|e| CheckError::StorageError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TupleWrite};
    use kraalzibar_storage::InMemoryStore;

    #[tokio::test]
    async fn adapter_reads_tuples_from_store() {
        let store = Arc::new(InMemoryStore::new());
        store
            .write(
                &[TupleWrite::new(
                    ObjectRef::new("document", "readme"),
                    "viewer",
                    SubjectRef::direct("user", "alice"),
                )],
                &[],
            )
            .await
            .unwrap();

        let adapter = StoreTupleReader::new(Arc::clone(&store));
        let filter = TupleFilter {
            object_type: Some("document".to_string()),
            ..Default::default()
        };
        let tuples = adapter.read_tuples(&filter, None).await.unwrap();

        assert_eq!(tuples.len(), 1);
        assert_eq!(tuples[0].object.object_type, "document");
        assert_eq!(tuples[0].subject.subject_id, "alice");
    }

    #[tokio::test]
    async fn adapter_maps_storage_error_to_check_error() {
        let store = Arc::new(InMemoryStore::new());
        let adapter = StoreTupleReader::new(Arc::clone(&store));

        let filter = TupleFilter {
            object_type: Some("document".to_string()),
            ..Default::default()
        };
        let result = adapter
            .read_tuples(&filter, Some(SnapshotToken::new(999)))
            .await;

        let err = result.unwrap_err();
        assert!(
            matches!(err, CheckError::StorageError(ref msg) if msg.contains("ahead")),
            "expected StorageError with 'ahead', got: {err}"
        );
    }

    #[tokio::test]
    async fn adapter_passes_snapshot_token() {
        let store = Arc::new(InMemoryStore::new());

        let token = store
            .write(
                &[TupleWrite::new(
                    ObjectRef::new("document", "readme"),
                    "viewer",
                    SubjectRef::direct("user", "alice"),
                )],
                &[],
            )
            .await
            .unwrap();

        store
            .write(
                &[TupleWrite::new(
                    ObjectRef::new("document", "readme"),
                    "editor",
                    SubjectRef::direct("user", "bob"),
                )],
                &[],
            )
            .await
            .unwrap();

        let adapter = StoreTupleReader::new(Arc::clone(&store));
        let filter = TupleFilter::default();
        let tuples = adapter.read_tuples(&filter, Some(token)).await.unwrap();

        assert_eq!(tuples.len(), 1, "should only see tuples at snapshot");
        assert_eq!(tuples[0].subject.subject_id, "alice");
    }
}
