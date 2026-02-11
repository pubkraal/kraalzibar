use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_stream::StreamExt;
use tonic::transport::{Channel, Server};

use kraalzibar_core::engine::EngineConfig;
use kraalzibar_core::schema::SchemaLimits;
use kraalzibar_server::grpc::{PermissionServiceImpl, RelationshipServiceImpl, SchemaServiceImpl};
use kraalzibar_server::middleware::AuthState;
use kraalzibar_server::proto::kraalzibar::v1::{
    self as v1, permission_service_client::PermissionServiceClient,
    permission_service_server::PermissionServiceServer,
    relationship_service_client::RelationshipServiceClient,
    relationship_service_server::RelationshipServiceServer,
    schema_service_client::SchemaServiceClient, schema_service_server::SchemaServiceServer,
};
use kraalzibar_server::service::AuthzService;
use kraalzibar_storage::InMemoryStoreFactory;

const SCHEMA: &str = r#"
    definition user {}

    definition document {
        relation owner: user
        relation editor: user
        relation viewer: user

        permission can_edit = owner + editor
        permission can_view = can_edit + viewer
    }
"#;

async fn start_test_server() -> (String, tokio::sync::oneshot::Sender<()>) {
    let factory = Arc::new(InMemoryStoreFactory::new());
    let service = Arc::new(AuthzService::new(
        factory,
        EngineConfig::default(),
        SchemaLimits::default(),
    ));
    let auth_state = AuthState::dev_mode();

    let grpc_auth = {
        let auth = auth_state.clone();
        move |req: tonic::Request<()>| -> Result<tonic::Request<()>, tonic::Status> {
            kraalzibar_server::middleware::grpc_auth_interceptor(&auth, req)
        }
    };

    let permission_svc = tonic::service::interceptor::InterceptedService::new(
        PermissionServiceServer::new(PermissionServiceImpl::new(Arc::clone(&service))),
        grpc_auth.clone(),
    );
    let relationship_svc = tonic::service::interceptor::InterceptedService::new(
        RelationshipServiceServer::new(RelationshipServiceImpl::new(Arc::clone(&service))),
        grpc_auth.clone(),
    );
    let schema_svc = tonic::service::interceptor::InterceptedService::new(
        SchemaServiceServer::new(SchemaServiceImpl::new(Arc::clone(&service))),
        grpc_auth,
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = format!("http://{addr}");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(permission_svc)
            .add_service(relationship_svc)
            .add_service(schema_svc)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    // Give the server a moment to start accepting connections
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (addr_str, shutdown_tx)
}

async fn connect(
    addr: &str,
) -> (
    SchemaServiceClient<Channel>,
    RelationshipServiceClient<Channel>,
    PermissionServiceClient<Channel>,
) {
    let channel = Channel::from_shared(addr.to_string())
        .unwrap()
        .connect()
        .await
        .unwrap();

    (
        SchemaServiceClient::new(channel.clone()),
        RelationshipServiceClient::new(channel.clone()),
        PermissionServiceClient::new(channel),
    )
}

fn touch_update(
    obj_type: &str,
    obj_id: &str,
    relation: &str,
    subj_type: &str,
    subj_id: &str,
) -> v1::RelationshipUpdate {
    v1::RelationshipUpdate {
        operation: v1::relationship_update::Operation::Touch as i32,
        relationship: Some(v1::Relationship {
            object: Some(v1::ObjectReference {
                object_type: obj_type.to_string(),
                object_id: obj_id.to_string(),
            }),
            relation: relation.to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: subj_type.to_string(),
                subject_id: subj_id.to_string(),
                subject_relation: None,
            }),
        }),
    }
}

fn full_consistency() -> Option<v1::Consistency> {
    Some(v1::Consistency {
        requirement: Some(v1::consistency::Requirement::FullConsistency(true)),
    })
}

fn at_exact_snapshot(token: &str) -> Option<v1::Consistency> {
    Some(v1::Consistency {
        requirement: Some(v1::consistency::Requirement::AtExactSnapshot(
            v1::ZedToken {
                token: token.to_string(),
            },
        )),
    })
}

#[tokio::test]
async fn write_and_read_schema() {
    let (addr, _shutdown) = start_test_server().await;
    let (mut schema_client, _, _) = connect(&addr).await;

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    let response = schema_client
        .read_schema(v1::ReadSchemaRequest {})
        .await
        .unwrap();

    assert!(response.into_inner().schema.contains("document"));
}

#[tokio::test]
async fn write_and_read_relationships() {
    let (addr, _shutdown) = start_test_server().await;
    let (_, mut rel_client, _) = connect(&addr).await;

    let write_resp = rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![touch_update(
                "document", "readme", "viewer", "user", "alice",
            )],
        })
        .await
        .unwrap()
        .into_inner();

    assert!(
        write_resp.written_at.is_some(),
        "write should return a token"
    );

    let read_resp = rel_client
        .read_relationships(v1::ReadRelationshipsRequest {
            consistency: full_consistency(),
            filter: Some(v1::RelationshipFilter {
                object_type: Some("document".to_string()),
                ..Default::default()
            }),
            optional_limit: 0,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(read_resp.relationships.len(), 1);
    let rel = &read_resp.relationships[0];
    assert_eq!(rel.subject.as_ref().unwrap().subject_id, "alice");
}

#[tokio::test]
async fn check_permission_allowed_and_denied() {
    let (addr, _shutdown) = start_test_server().await;
    let (mut schema_client, mut rel_client, mut perm_client) = connect(&addr).await;

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![touch_update(
                "document", "readme", "viewer", "user", "alice",
            )],
        })
        .await
        .unwrap();

    // alice should be allowed
    let resp = perm_client
        .check_permission(v1::CheckPermissionRequest {
            consistency: full_consistency(),
            resource: Some(v1::ObjectReference {
                object_type: "document".to_string(),
                object_id: "readme".to_string(),
            }),
            permission: "can_view".to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: "user".to_string(),
                subject_id: "alice".to_string(),
                subject_relation: None,
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.permissionship,
        v1::check_permission_response::Permissionship::HasPermission as i32,
    );
    assert!(resp.checked_at.is_some(), "should return checked_at token");

    // bob should be denied
    let resp = perm_client
        .check_permission(v1::CheckPermissionRequest {
            consistency: full_consistency(),
            resource: Some(v1::ObjectReference {
                object_type: "document".to_string(),
                object_id: "readme".to_string(),
            }),
            permission: "can_view".to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: "user".to_string(),
                subject_id: "bob".to_string(),
                subject_relation: None,
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.permissionship,
        v1::check_permission_response::Permissionship::NoPermission as i32,
    );
}

#[tokio::test]
async fn expand_permission_tree() {
    let (addr, _shutdown) = start_test_server().await;
    let (mut schema_client, mut rel_client, mut perm_client) = connect(&addr).await;

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![touch_update(
                "document", "readme", "viewer", "user", "alice",
            )],
        })
        .await
        .unwrap();

    let resp = perm_client
        .expand_permission_tree(v1::ExpandPermissionTreeRequest {
            consistency: full_consistency(),
            resource: Some(v1::ObjectReference {
                object_type: "document".to_string(),
                object_id: "readme".to_string(),
            }),
            permission: "can_view".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.tree.is_some(), "expand should return a tree");
    assert!(
        resp.expanded_at.is_some(),
        "expand should return expanded_at"
    );
}

#[tokio::test]
async fn lookup_resources() {
    let (addr, _shutdown) = start_test_server().await;
    let (mut schema_client, mut rel_client, mut perm_client) = connect(&addr).await;

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![
                touch_update("document", "readme", "viewer", "user", "alice"),
                touch_update("document", "design", "viewer", "user", "alice"),
                touch_update("document", "secret", "viewer", "user", "bob"),
            ],
        })
        .await
        .unwrap();

    let resp = perm_client
        .lookup_resources(v1::LookupResourcesRequest {
            consistency: full_consistency(),
            resource_type: "document".to_string(),
            permission: "can_view".to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: "user".to_string(),
                subject_id: "alice".to_string(),
                subject_relation: None,
            }),
            optional_limit: 0,
        })
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let mut resource_ids = Vec::new();
    while let Some(item) = stream.next().await {
        let item = item.unwrap();
        resource_ids.push(item.resource_id);
    }

    resource_ids.sort();
    assert_eq!(resource_ids, vec!["design", "readme"]);
}

#[tokio::test]
async fn lookup_subjects() {
    let (addr, _shutdown) = start_test_server().await;
    let (mut schema_client, mut rel_client, mut perm_client) = connect(&addr).await;

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![
                touch_update("document", "readme", "viewer", "user", "alice"),
                touch_update("document", "readme", "editor", "user", "bob"),
            ],
        })
        .await
        .unwrap();

    let resp = perm_client
        .lookup_subjects(v1::LookupSubjectsRequest {
            consistency: full_consistency(),
            resource: Some(v1::ObjectReference {
                object_type: "document".to_string(),
                object_id: "readme".to_string(),
            }),
            permission: "can_view".to_string(),
            subject_type: "user".to_string(),
        })
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let mut subject_ids = Vec::new();
    while let Some(item) = stream.next().await {
        let item = item.unwrap();
        subject_ids.push(item.subject.unwrap().subject_id);
    }

    subject_ids.sort();
    assert_eq!(subject_ids, vec!["alice", "bob"]);
}

#[tokio::test]
async fn consistency_token_flow() {
    let (addr, _shutdown) = start_test_server().await;
    let (_, mut rel_client, _) = connect(&addr).await;

    let token1 = rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![touch_update(
                "document", "readme", "viewer", "user", "alice",
            )],
        })
        .await
        .unwrap()
        .into_inner()
        .written_at
        .unwrap()
        .token;

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![touch_update("document", "design", "viewer", "user", "bob")],
        })
        .await
        .unwrap();

    // Reading at token1 should only see the first write
    let read_resp = rel_client
        .read_relationships(v1::ReadRelationshipsRequest {
            consistency: at_exact_snapshot(&token1),
            filter: None,
            optional_limit: 0,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        read_resp.relationships.len(),
        1,
        "at_exact_snapshot(token1) should only see the first write"
    );
    assert_eq!(
        read_resp.relationships[0]
            .subject
            .as_ref()
            .unwrap()
            .subject_id,
        "alice"
    );
}

#[tokio::test]
async fn error_check_without_schema() {
    let (addr, _shutdown) = start_test_server().await;
    let (_, _, mut perm_client) = connect(&addr).await;

    let result = perm_client
        .check_permission(v1::CheckPermissionRequest {
            consistency: full_consistency(),
            resource: Some(v1::ObjectReference {
                object_type: "document".to_string(),
                object_id: "readme".to_string(),
            }),
            permission: "can_view".to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: "user".to_string(),
                subject_id: "alice".to_string(),
                subject_relation: None,
            }),
        })
        .await;

    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn error_missing_required_fields() {
    let (addr, _shutdown) = start_test_server().await;
    let (_, _, mut perm_client) = connect(&addr).await;

    let result = perm_client
        .check_permission(v1::CheckPermissionRequest {
            consistency: full_consistency(),
            resource: None, // missing required field
            permission: "can_view".to_string(),
            subject: Some(v1::SubjectReference {
                subject_type: "user".to_string(),
                subject_id: "alice".to_string(),
                subject_relation: None,
            }),
        })
        .await;

    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}
