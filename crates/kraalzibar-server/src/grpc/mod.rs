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
        ApiError::Storage(kraalzibar_storage::StorageError::DuplicateTuple) => {
            Status::already_exists(err.to_string())
        }
        ApiError::Storage(kraalzibar_storage::StorageError::EmptyDeleteFilter)
        | ApiError::Storage(kraalzibar_storage::StorageError::SnapshotAhead { .. }) => {
            Status::invalid_argument(err.to_string())
        }
        ApiError::Check(CheckError::StorageError(_))
        | ApiError::Storage(kraalzibar_storage::StorageError::Internal(_)) => {
            tracing::error!(error = %err, "internal storage error");
            Status::internal("internal server error")
        }
        ApiError::Parse(_) | ApiError::Validation(_) => Status::invalid_argument(err.to_string()),
        ApiError::BreakingChanges(_) => Status::failed_precondition(err.to_string()),
        ApiError::SchemaNotFound => Status::not_found(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiError;
    use kraalzibar_storage::StorageError;

    #[test]
    fn duplicate_tuple_maps_to_already_exists() {
        let err = ApiError::Storage(StorageError::DuplicateTuple);
        let status = api_error_to_status(err);

        assert_eq!(status.code(), tonic::Code::AlreadyExists);
        assert!(
            status.message().contains("duplicate"),
            "expected 'duplicate' in message, got: {}",
            status.message()
        );
    }

    #[test]
    fn empty_delete_filter_maps_to_invalid_argument() {
        let err = ApiError::Storage(StorageError::EmptyDeleteFilter);
        let status = api_error_to_status(err);

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(
            status.message().contains("delete filter"),
            "expected 'delete filter' in message, got: {}",
            status.message()
        );
    }

    #[test]
    fn snapshot_ahead_maps_to_invalid_argument() {
        let err = ApiError::Storage(StorageError::SnapshotAhead {
            requested: 10,
            current: 5,
        });
        let status = api_error_to_status(err);

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(
            status.message().contains("ahead"),
            "expected 'ahead' in message, got: {}",
            status.message()
        );
    }

    #[test]
    fn internal_storage_error_maps_to_internal_with_generic_message() {
        let err = ApiError::Storage(StorageError::Internal("db connection lost".to_string()));
        let status = api_error_to_status(err);

        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), "internal server error");
        assert!(
            !status.message().contains("db connection"),
            "internal details must not leak to client"
        );
    }
}
