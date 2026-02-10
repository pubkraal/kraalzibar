use std::sync::Arc;

use tonic::{Request, Response, Status};

use kraalzibar_core::tuple::TenantId;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::metrics::Metrics;
use crate::proto::kraalzibar::v1::{self, schema_service_server::SchemaService};
use crate::service::AuthzService;

pub struct SchemaServiceImpl<F: StoreFactory> {
    service: Arc<AuthzService<F>>,
    metrics: Option<Arc<Metrics>>,
}

impl<F: StoreFactory> SchemaServiceImpl<F> {
    pub fn new(service: Arc<AuthzService<F>>) -> Self {
        Self {
            service,
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

#[allow(clippy::result_large_err)]
fn extract_tenant_id<T>(request: &Request<T>) -> Result<TenantId, Status> {
    request
        .extensions()
        .get::<TenantId>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("missing tenant context"))
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
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let result = self
            .service
            .write_schema(&tenant_id, &req.schema, req.force)
            .await
            .map_err(super::api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("write_schema", start.elapsed());
        }

        Ok(Response::new(v1::WriteSchemaResponse {
            written_at: None,
            breaking_changes_overridden: result.breaking_changes_overridden,
        }))
    }

    async fn read_schema(
        &self,
        request: Request<v1::ReadSchemaRequest>,
    ) -> Result<Response<v1::ReadSchemaResponse>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let schema = self
            .service
            .read_schema(&tenant_id)
            .await
            .map_err(super::api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("read_schema", start.elapsed());
        }

        match schema {
            Some(text) => Ok(Response::new(v1::ReadSchemaResponse { schema: text })),
            None => Err(Status::not_found("no schema has been written")),
        }
    }
}
