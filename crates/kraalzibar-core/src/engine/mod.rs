mod check;
mod expand;

pub use check::{CheckEngine, CheckRequest, CheckResult};
pub use expand::{ExpandEngine, ExpandRequest, ExpandTree};

use std::future::Future;

use crate::tuple::{SnapshotToken, Tuple, TupleFilter};

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("type not found: {0}")]
    TypeNotFound(String),

    #[error("permission '{permission}' not found on type '{type_name}'")]
    PermissionNotFound {
        type_name: String,
        permission: String,
    },

    #[error("relation '{relation}' not found on type '{type_name}'")]
    RelationNotFound { type_name: String, relation: String },

    #[error("max depth exceeded: {0}")]
    MaxDepthExceeded(usize),

    #[error("too many results: expand produced {0} tuples, limit is {1}")]
    TooManyResults(usize, usize),

    #[error("storage error: {0}")]
    StorageError(String),
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_depth: usize,
    pub max_concurrent_branches: usize,
    pub max_expand_results: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_depth: 6,
            max_concurrent_branches: 10,
            max_expand_results: 10_000,
        }
    }
}

pub trait TupleReader: Send + Sync {
    fn read_tuples(
        &self,
        filter: &TupleFilter,
        snapshot: Option<SnapshotToken>,
    ) -> impl Future<Output = Result<Vec<Tuple>, CheckError>> + Send;
}
