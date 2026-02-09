use std::sync::Arc;

use tonic::{Request, Response, Status};

use kraalzibar_core::tuple::TenantId;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::proto::kraalzibar::v1::{self, schema_service_server::SchemaService};
use crate::service::AuthzService;

pub struct SchemaServiceImpl<F: StoreFactory> {
    service: Arc<AuthzService<F>>,
    tenant_id: TenantId,
}

impl<F: StoreFactory> SchemaServiceImpl<F> {
    pub fn new(service: Arc<AuthzService<F>>, tenant_id: TenantId) -> Self {
        Self { service, tenant_id }
    }
}

#[tonic::async_trait]
impl<F> SchemaService for SchemaServiceImpl<F>
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    async fn write_schema(
        &self,
        request: Request<v1::WriteSchemaRequest>,
    ) -> Result<Response<v1::WriteSchemaResponse>, Status> {
        let req = request.into_inner();

        let result = self
            .service
            .write_schema(&self.tenant_id, &req.schema, req.force)
            .await
            .map_err(|e| match &e {
                crate::error::ApiError::Parse(_) | crate::error::ApiError::Validation(_) => {
                    Status::invalid_argument(e.to_string())
                }
                crate::error::ApiError::BreakingChanges(_) => {
                    Status::failed_precondition(e.to_string())
                }
                _ => Status::internal(e.to_string()),
            })?;

        Ok(Response::new(v1::WriteSchemaResponse {
            written_at: None,
            breaking_changes_overridden: result.breaking_changes_overridden,
        }))
    }

    async fn read_schema(
        &self,
        _request: Request<v1::ReadSchemaRequest>,
    ) -> Result<Response<v1::ReadSchemaResponse>, Status> {
        let schema = self
            .service
            .read_schema(&self.tenant_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match schema {
            Some(text) => Ok(Response::new(v1::ReadSchemaResponse { schema: text })),
            None => Err(Status::not_found("no schema has been written")),
        }
    }
}
