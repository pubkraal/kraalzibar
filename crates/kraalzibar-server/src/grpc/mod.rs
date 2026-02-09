mod conversions;
mod permission_service;
mod relationship_service;
mod schema_service;

pub use permission_service::PermissionServiceImpl;
pub use relationship_service::RelationshipServiceImpl;
pub use schema_service::SchemaServiceImpl;

use tonic::Status;

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
