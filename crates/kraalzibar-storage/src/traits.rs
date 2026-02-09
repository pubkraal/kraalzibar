use kraalzibar_core::tuple::{SnapshotToken, TenantId, Tuple, TupleFilter, TupleWrite};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    #[error("duplicate tuple in write batch")]
    DuplicateTuple,
    #[error("snapshot {requested} is ahead of current {current}")]
    SnapshotAhead { requested: u64, current: u64 },
    #[error("internal storage error: {0}")]
    Internal(String),
}

pub trait RelationshipStore: Send + Sync {
    fn write(
        &self,
        writes: &[TupleWrite],
        deletes: &[TupleFilter],
    ) -> impl Future<Output = Result<SnapshotToken, StorageError>> + Send;

    fn read(
        &self,
        filter: &TupleFilter,
        snapshot: Option<SnapshotToken>,
    ) -> impl Future<Output = Result<Vec<Tuple>, StorageError>> + Send;

    fn snapshot(&self) -> impl Future<Output = Result<SnapshotToken, StorageError>> + Send;
}

pub trait SchemaStore: Send + Sync {
    fn write_schema(
        &self,
        definition: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    fn read_schema(&self) -> impl Future<Output = Result<Option<String>, StorageError>> + Send;
}

pub trait StoreFactory: Send + Sync {
    type Store: RelationshipStore + SchemaStore;

    fn for_tenant(&self, tenant_id: &TenantId) -> Self::Store;
}
