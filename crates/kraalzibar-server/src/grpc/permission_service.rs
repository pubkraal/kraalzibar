use std::sync::Arc;

use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::metrics::Metrics;
use crate::proto::kraalzibar::v1::{self, permission_service_server::PermissionService};
use crate::service::{
    AuthzService, CheckPermissionInput, ExpandPermissionInput, LookupResourcesInput,
    LookupSubjectsInput,
};

use super::{conversions, extract_tenant_id};

pub struct PermissionServiceImpl<F: StoreFactory> {
    service: Arc<AuthzService<F>>,
    metrics: Option<Arc<Metrics>>,
}

impl<F: StoreFactory> PermissionServiceImpl<F> {
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
impl<F> PermissionService for PermissionServiceImpl<F>
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    async fn check_permission(
        &self,
        request: Request<v1::CheckPermissionRequest>,
    ) -> Result<Response<v1::CheckPermissionResponse>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let resource = req
            .resource
            .ok_or_else(|| Status::invalid_argument("resource is required"))?;
        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;

        let input = CheckPermissionInput {
            object_type: resource.object_type,
            object_id: resource.object_id,
            permission: req.permission,
            subject_type: subject.subject_type,
            subject_id: subject.subject_id,
            consistency: conversions::proto_consistency_to_domain(req.consistency.as_ref())?,
        };

        let result = self
            .service
            .check_permission(&tenant_id, input)
            .await
            .map_err(api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("check", start.elapsed());
        }

        let permissionship = if result.allowed {
            v1::check_permission_response::Permissionship::HasPermission
        } else {
            v1::check_permission_response::Permissionship::NoPermission
        };

        Ok(Response::new(v1::CheckPermissionResponse {
            checked_at: conversions::snapshot_to_zed_token(result.snapshot),
            permissionship: permissionship as i32,
        }))
    }

    async fn expand_permission_tree(
        &self,
        request: Request<v1::ExpandPermissionTreeRequest>,
    ) -> Result<Response<v1::ExpandPermissionTreeResponse>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let resource = req
            .resource
            .ok_or_else(|| Status::invalid_argument("resource is required"))?;

        let input = ExpandPermissionInput {
            object_type: resource.object_type,
            object_id: resource.object_id,
            permission: req.permission,
            consistency: conversions::proto_consistency_to_domain(req.consistency.as_ref())?,
        };

        let output = self
            .service
            .expand_permission_tree(&tenant_id, input)
            .await
            .map_err(api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("expand", start.elapsed());
        }

        Ok(Response::new(v1::ExpandPermissionTreeResponse {
            expanded_at: conversions::snapshot_to_zed_token(output.snapshot),
            tree: Some(conversions::domain_expand_tree_to_proto(&output.tree)),
        }))
    }

    type LookupResourcesStream = ReceiverStream<Result<v1::LookupResourcesResponse, Status>>;

    async fn lookup_resources(
        &self,
        request: Request<v1::LookupResourcesRequest>,
    ) -> Result<Response<Self::LookupResourcesStream>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let subject = req
            .subject
            .ok_or_else(|| Status::invalid_argument("subject is required"))?;

        let input = LookupResourcesInput {
            resource_type: req.resource_type,
            permission: req.permission,
            subject_type: subject.subject_type,
            subject_id: subject.subject_id,
            consistency: conversions::proto_consistency_to_domain(req.consistency.as_ref())?,
            limit: if req.optional_limit > 0 {
                Some((req.optional_limit as usize).min(10_000))
            } else {
                None
            },
        };

        let output = self
            .service
            .lookup_resources(&tenant_id, input)
            .await
            .map_err(api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("lookup_resources", start.elapsed());
        }

        let looked_up_at = conversions::snapshot_to_zed_token(output.snapshot);
        let (tx, rx) = tokio::sync::mpsc::channel(output.resource_ids.len().max(1));

        for resource_id in output.resource_ids {
            let _ = tx
                .send(Ok(v1::LookupResourcesResponse {
                    looked_up_at: looked_up_at.clone(),
                    resource_id,
                }))
                .await;
        }

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    type LookupSubjectsStream = ReceiverStream<Result<v1::LookupSubjectsResponse, Status>>;

    async fn lookup_subjects(
        &self,
        request: Request<v1::LookupSubjectsRequest>,
    ) -> Result<Response<Self::LookupSubjectsStream>, Status> {
        let start = std::time::Instant::now();
        let tenant_id = extract_tenant_id(&request)?;
        let req = request.into_inner();

        let resource = req
            .resource
            .ok_or_else(|| Status::invalid_argument("resource is required"))?;

        let input = LookupSubjectsInput {
            object_type: resource.object_type,
            object_id: resource.object_id,
            permission: req.permission,
            subject_type: req.subject_type,
            consistency: conversions::proto_consistency_to_domain(req.consistency.as_ref())?,
        };

        let output = self
            .service
            .lookup_subjects(&tenant_id, input)
            .await
            .map_err(api_error_to_status)?;

        if let Some(m) = &self.metrics {
            m.record_method_request("lookup_subjects", start.elapsed());
        }

        let looked_up_at = conversions::snapshot_to_zed_token(output.snapshot);
        let (tx, rx) = tokio::sync::mpsc::channel(output.subjects.len().max(1));

        for subject in &output.subjects {
            let _ = tx
                .send(Ok(v1::LookupSubjectsResponse {
                    looked_up_at: looked_up_at.clone(),
                    subject: Some(conversions::domain_subject_to_proto(subject)),
                }))
                .await;
        }

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

use super::api_error_to_status;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_maps_type_not_found_to_not_found() {
        use kraalzibar_core::engine::CheckError;
        let err = crate::error::ApiError::Check(CheckError::TypeNotFound("doc".to_string()));
        let status = api_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn api_error_maps_parse_error_to_invalid_argument() {
        use kraalzibar_core::schema::ParseError;
        let err = crate::error::ApiError::Parse(ParseError::MixedOperators);
        let status = api_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn api_error_maps_breaking_changes_to_failed_precondition() {
        let err = crate::error::ApiError::BreakingChanges(vec![]);
        let status = api_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}
