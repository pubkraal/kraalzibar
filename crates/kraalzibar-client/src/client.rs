use kraalzibar_core::tuple::{ObjectRef, SnapshotToken, SubjectRef, TupleFilter};
use tokio_stream::StreamExt;
use tonic::transport::Channel;

use crate::config::ClientOptions;
use crate::conversions::{self, Consistency};
use crate::error::ClientError;
use crate::interceptor::ApiKeyInterceptor;
use crate::proto::kraalzibar::v1::{
    self, permission_service_client::PermissionServiceClient,
    relationship_service_client::RelationshipServiceClient,
    schema_service_client::SchemaServiceClient,
};

type InterceptedChannel =
    tonic::service::interceptor::InterceptedService<Channel, ApiKeyInterceptor>;

pub struct KraalzibarClient {
    permissions: PermissionServiceClient<InterceptedChannel>,
    relationships: RelationshipServiceClient<InterceptedChannel>,
    schemas: SchemaServiceClient<InterceptedChannel>,
}

impl KraalzibarClient {
    pub async fn connect(endpoint: &str, options: ClientOptions) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(endpoint.to_string())
            .map_err(|e| ClientError::Connection(e.to_string()))?
            .connect_timeout(options.connect_timeout)
            .timeout(options.timeout)
            .connect()
            .await
            .map_err(|e| ClientError::Connection(e.to_string()))?;

        Ok(Self::from_channel(channel, options))
    }

    pub fn from_channel(channel: Channel, options: ClientOptions) -> Self {
        let interceptor = ApiKeyInterceptor::new(options.api_key);
        Self {
            permissions: PermissionServiceClient::with_interceptor(
                channel.clone(),
                interceptor.clone(),
            ),
            relationships: RelationshipServiceClient::with_interceptor(
                channel.clone(),
                interceptor.clone(),
            ),
            schemas: SchemaServiceClient::with_interceptor(channel, interceptor),
        }
    }

    pub async fn check_permission(
        &mut self,
        request: CheckPermissionRequest,
    ) -> Result<CheckPermissionResponse, ClientError> {
        let proto_req = v1::CheckPermissionRequest {
            consistency: Some(conversions::consistency_to_proto(&request.consistency)),
            resource: Some(conversions::domain_object_to_proto(&request.resource)),
            permission: request.permission,
            subject: Some(conversions::domain_subject_to_proto(&request.subject)),
        };

        let response = self
            .permissions
            .check_permission(proto_req)
            .await?
            .into_inner();

        let allowed = response.permissionship
            == v1::check_permission_response::Permissionship::HasPermission as i32;
        let checked_at = response
            .checked_at
            .as_ref()
            .and_then(conversions::zed_token_to_snapshot);

        Ok(CheckPermissionResponse {
            allowed,
            checked_at,
        })
    }

    pub async fn write_relationships(
        &mut self,
        updates: Vec<RelationshipUpdate>,
    ) -> Result<WriteRelationshipsResponse, ClientError> {
        let proto_updates: Vec<v1::RelationshipUpdate> = updates
            .into_iter()
            .map(|u| {
                let operation = match u.operation {
                    RelationshipOperation::Touch => {
                        v1::relationship_update::Operation::Touch as i32
                    }
                    RelationshipOperation::Delete => {
                        v1::relationship_update::Operation::Delete as i32
                    }
                };
                v1::RelationshipUpdate {
                    operation,
                    relationship: Some(v1::Relationship {
                        object: Some(conversions::domain_object_to_proto(&u.object)),
                        relation: u.relation,
                        subject: Some(conversions::domain_subject_to_proto(&u.subject)),
                    }),
                }
            })
            .collect();

        let response = self
            .relationships
            .write_relationships(v1::WriteRelationshipsRequest {
                updates: proto_updates,
            })
            .await?
            .into_inner();

        let written_at = response
            .written_at
            .as_ref()
            .and_then(conversions::zed_token_to_snapshot);

        Ok(WriteRelationshipsResponse { written_at })
    }

    pub async fn read_relationships(
        &mut self,
        request: ReadRelationshipsRequest,
    ) -> Result<Vec<Relationship>, ClientError> {
        let proto_req = v1::ReadRelationshipsRequest {
            consistency: Some(conversions::consistency_to_proto(&request.consistency)),
            filter: Some(conversions::domain_filter_to_proto(&request.filter)),
            optional_limit: request.limit.unwrap_or(0) as u32,
        };

        let response = self
            .relationships
            .read_relationships(proto_req)
            .await?
            .into_inner();

        let relationships = response
            .relationships
            .iter()
            .filter_map(|rel| {
                let object = rel
                    .object
                    .as_ref()
                    .map(conversions::proto_object_to_domain)?;
                let subject = rel
                    .subject
                    .as_ref()
                    .map(conversions::proto_subject_to_domain)?;
                Some(Relationship {
                    object,
                    relation: rel.relation.clone(),
                    subject,
                })
            })
            .collect();

        Ok(relationships)
    }

    pub async fn lookup_resources(
        &mut self,
        request: LookupResourcesRequest,
    ) -> Result<Vec<String>, ClientError> {
        let proto_req = v1::LookupResourcesRequest {
            consistency: Some(conversions::consistency_to_proto(&request.consistency)),
            resource_type: request.resource_type,
            permission: request.permission,
            subject: Some(conversions::domain_subject_to_proto(&request.subject)),
            optional_limit: request.limit.unwrap_or(0) as u32,
        };

        let mut stream = self
            .permissions
            .lookup_resources(proto_req)
            .await?
            .into_inner();

        let mut resource_ids = Vec::new();
        while let Some(response) = stream.next().await {
            let response = response?;
            resource_ids.push(response.resource_id);
        }

        Ok(resource_ids)
    }

    pub async fn lookup_subjects(
        &mut self,
        request: LookupSubjectsRequest,
    ) -> Result<Vec<SubjectRef>, ClientError> {
        let proto_req = v1::LookupSubjectsRequest {
            consistency: Some(conversions::consistency_to_proto(&request.consistency)),
            resource: Some(conversions::domain_object_to_proto(&request.resource)),
            permission: request.permission,
            subject_type: request.subject_type,
        };

        let mut stream = self
            .permissions
            .lookup_subjects(proto_req)
            .await?
            .into_inner();

        let mut subjects = Vec::new();
        while let Some(response) = stream.next().await {
            let response = response?;
            if let Some(subj) = response.subject.as_ref() {
                subjects.push(conversions::proto_subject_to_domain(subj));
            }
        }

        Ok(subjects)
    }

    pub async fn write_schema(
        &mut self,
        schema: &str,
        force: bool,
    ) -> Result<WriteSchemaResponse, ClientError> {
        let response = self
            .schemas
            .write_schema(v1::WriteSchemaRequest {
                schema: schema.to_string(),
                force,
            })
            .await?
            .into_inner();

        let written_at = response
            .written_at
            .as_ref()
            .and_then(conversions::zed_token_to_snapshot);

        Ok(WriteSchemaResponse {
            written_at,
            breaking_changes_overridden: response.breaking_changes_overridden,
        })
    }

    pub async fn read_schema(&mut self) -> Result<Option<String>, ClientError> {
        let response = self
            .schemas
            .read_schema(v1::ReadSchemaRequest {})
            .await?
            .into_inner();

        if response.schema.is_empty() {
            Ok(None)
        } else {
            Ok(Some(response.schema))
        }
    }

    pub async fn expand_permission_tree(
        &mut self,
        request: ExpandPermissionTreeRequest,
    ) -> Result<ExpandPermissionTreeResponse, ClientError> {
        let proto_req = v1::ExpandPermissionTreeRequest {
            consistency: Some(conversions::consistency_to_proto(&request.consistency)),
            resource: Some(conversions::domain_object_to_proto(&request.resource)),
            permission: request.permission,
        };

        let response = self
            .permissions
            .expand_permission_tree(proto_req)
            .await?
            .into_inner();

        let expanded_at = response
            .expanded_at
            .as_ref()
            .and_then(conversions::zed_token_to_snapshot);

        Ok(ExpandPermissionTreeResponse {
            expanded_at,
            tree: response.tree,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CheckPermissionRequest {
    pub resource: ObjectRef,
    pub permission: String,
    pub subject: SubjectRef,
    pub consistency: Consistency,
}

#[derive(Debug, Clone)]
pub struct CheckPermissionResponse {
    pub allowed: bool,
    pub checked_at: Option<SnapshotToken>,
}

#[derive(Debug, Clone)]
pub enum RelationshipOperation {
    Touch,
    Delete,
}

#[derive(Debug, Clone)]
pub struct RelationshipUpdate {
    pub operation: RelationshipOperation,
    pub object: ObjectRef,
    pub relation: String,
    pub subject: SubjectRef,
}

#[derive(Debug, Clone)]
pub struct WriteRelationshipsResponse {
    pub written_at: Option<SnapshotToken>,
}

#[derive(Debug, Clone)]
pub struct ReadRelationshipsRequest {
    pub filter: TupleFilter,
    pub consistency: Consistency,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub object: ObjectRef,
    pub relation: String,
    pub subject: SubjectRef,
}

#[derive(Debug, Clone)]
pub struct LookupResourcesRequest {
    pub resource_type: String,
    pub permission: String,
    pub subject: SubjectRef,
    pub consistency: Consistency,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct LookupSubjectsRequest {
    pub resource: ObjectRef,
    pub permission: String,
    pub subject_type: String,
    pub consistency: Consistency,
}

#[derive(Debug, Clone)]
pub struct WriteSchemaResponse {
    pub written_at: Option<SnapshotToken>,
    pub breaking_changes_overridden: bool,
}

#[derive(Debug, Clone)]
pub struct ExpandPermissionTreeRequest {
    pub resource: ObjectRef,
    pub permission: String,
    pub consistency: Consistency,
}

#[derive(Debug, Clone)]
pub struct ExpandPermissionTreeResponse {
    pub expanded_at: Option<SnapshotToken>,
    pub tree: Option<v1::PermissionExpansionTree>,
}
