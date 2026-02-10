use std::sync::Arc;

use tonic::{Request, Response, Status};

use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::metrics::Metrics;
use crate::proto::kraalzibar::v1::{self, relationship_service_server::RelationshipService};
use crate::service::AuthzService;

use super::{conversions, extract_tenant_id};

pub struct RelationshipServiceImpl<F: StoreFactory> {
    service: Arc<AuthzService<F>>,
    metrics: Option<Arc<Metrics>>,
}

impl<F: StoreFactory> RelationshipServiceImpl<F> {
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

#[tonic::async_trait]
impl<F> RelationshipService for RelationshipServiceImpl<F>
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    async fn write_relationships(
        &self,
        request: Request<v1::WriteRelationshipsRequest>,
    ) -> Result<Response<v1::WriteRelationshipsResponse>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let mut writes = Vec::new();
        let mut deletes = Vec::new();

        for update in &req.updates {
            let rel = update
                .relationship
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("relationship is required in update"))?;

            let op = v1::relationship_update::Operation::try_from(update.operation)
                .unwrap_or(v1::relationship_update::Operation::Unspecified);

            match op {
                v1::relationship_update::Operation::Touch => {
                    writes.push(conversions::proto_relationship_to_write(rel)?);
                }
                v1::relationship_update::Operation::Delete => {
                    deletes.push(conversions::proto_filter_to_domain(
                        &v1::RelationshipFilter {
                            object_type: rel.object.as_ref().map(|o| o.object_type.clone()),
                            object_id: rel.object.as_ref().map(|o| o.object_id.clone()),
                            relation: Some(rel.relation.clone()),
                            subject_type: rel.subject.as_ref().map(|s| s.subject_type.clone()),
                            subject_id: rel.subject.as_ref().map(|s| s.subject_id.clone()),
                        },
                    ));
                }
                v1::relationship_update::Operation::Unspecified => {
                    return Err(Status::invalid_argument("operation must be specified"));
                }
            }
        }

        let token = self
            .service
            .write_relationships(&tenant_id, &writes, &deletes)
            .await
            .map_err(super::api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("write_relationships", start.elapsed());
        }

        Ok(Response::new(v1::WriteRelationshipsResponse {
            written_at: conversions::snapshot_to_zed_token(Some(token)),
        }))
    }

    async fn read_relationships(
        &self,
        request: Request<v1::ReadRelationshipsRequest>,
    ) -> Result<Response<v1::ReadRelationshipsResponse>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let filter = req
            .filter
            .as_ref()
            .map(conversions::proto_filter_to_domain)
            .unwrap_or_default();

        let consistency = conversions::proto_consistency_to_domain(req.consistency.as_ref())?;

        let limit = if req.optional_limit > 0 {
            Some((req.optional_limit as usize).min(10_000))
        } else {
            Some(1000)
        };

        let tuples = self
            .service
            .read_relationships(&tenant_id, &filter, consistency, limit)
            .await
            .map_err(super::api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("read_relationships", start.elapsed());
        }

        let relationships: Vec<_> = tuples
            .iter()
            .map(conversions::domain_tuple_to_proto)
            .collect();

        Ok(Response::new(v1::ReadRelationshipsResponse {
            read_at: None,
            relationships,
        }))
    }

    type WatchStream = tokio_stream::wrappers::ReceiverStream<Result<v1::WatchResponse, Status>>;

    async fn watch(
        &self,
        _request: Request<v1::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        Err(Status::unimplemented("Watch is not yet implemented"))
    }
}
