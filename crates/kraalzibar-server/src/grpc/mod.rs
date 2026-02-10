mod conversions;
mod permission_service;
mod relationship_service;
mod schema_service;

pub use permission_service::PermissionServiceImpl;
pub use relationship_service::RelationshipServiceImpl;
pub use schema_service::SchemaServiceImpl;

use kraalzibar_core::tuple::TenantId;
use tonic::{Request, Status};

#[allow(clippy::result_large_err)]
fn extract_tenant_id<T>(request: &Request<T>) -> Result<TenantId, Status> {
    request
        .extensions()
        .get::<TenantId>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("missing tenant context"))
}

fn api_error_to_status(err: crate::error::ApiError) -> Status {
    use crate::error::ApiError;
    use kraalzibar_core::engine::CheckError;

    match &err {
        ApiError::Check(CheckError::TypeNotFound(_))
        | ApiError::Check(CheckError::PermissionNotFound { .. })
        | ApiError::Check(CheckError::RelationNotFound { .. }) => {
            Status::not_found(err.to_string())
        }
        ApiError::Check(CheckError::MaxDepthExceeded(_)) => {
            Status::resource_exhausted(err.to_string())
        }
        ApiError::Check(CheckError::StorageError(_)) | ApiError::Storage(_) => {
            tracing::error!(error = %err, "internal storage error");
            Status::internal("internal server error")
        }
        ApiError::Parse(_) | ApiError::Validation(_) => Status::invalid_argument(err.to_string()),
        ApiError::BreakingChanges(_) => Status::failed_precondition(err.to_string()),
        ApiError::SchemaNotFound => Status::not_found(err.to_string()),
    }
}
