use std::sync::Arc;

use kraalzibar_client::{
    CheckPermissionRequest, ClientOptions, Consistency, KraalzibarClient, LookupResourcesRequest,
    LookupSubjectsRequest, ReadRelationshipsRequest, RelationshipOperation, RelationshipUpdate,
};
use kraalzibar_core::engine::EngineConfig;
use kraalzibar_core::schema::SchemaLimits;
use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TenantId, TupleFilter};
use kraalzibar_server::grpc::{PermissionServiceImpl, RelationshipServiceImpl, SchemaServiceImpl};
use kraalzibar_server::proto::kraalzibar::v1::{
    permission_service_server::PermissionServiceServer,
    relationship_service_server::RelationshipServiceServer,
    schema_service_server::SchemaServiceServer,
};
use kraalzibar_server::service::AuthzService;
use kraalzibar_storage::InMemoryStoreFactory;

async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint = format!("http://{addr}");

    let factory = Arc::new(InMemoryStoreFactory::new());
    let tenant_id = TenantId::new(uuid::Uuid::nil());
    let service = Arc::new(AuthzService::new(
        factory,
        EngineConfig::default(),
        SchemaLimits::default(),
    ));

    let permission_svc = PermissionServiceServer::new(PermissionServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ));
    let relationship_svc = RelationshipServiceServer::new(RelationshipServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ));
    let schema_svc =
        SchemaServiceServer::new(SchemaServiceImpl::new(Arc::clone(&service), tenant_id));

    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(permission_svc)
            .add_service(relationship_svc)
            .add_service(schema_svc)
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (endpoint, handle)
}

async fn setup_client(endpoint: &str) -> KraalzibarClient {
    KraalzibarClient::connect(endpoint, ClientOptions::default())
        .await
        .unwrap()
}

async fn write_schema(client: &mut KraalzibarClient, schema: &str) {
    client.write_schema(schema, false).await.unwrap();
}

async fn write_touch(
    client: &mut KraalzibarClient,
    object_type: &str,
    object_id: &str,
    relation: &str,
    subject_type: &str,
    subject_id: &str,
) {
    client
        .write_relationships(vec![RelationshipUpdate {
            operation: RelationshipOperation::Touch,
            object: ObjectRef::new(object_type, object_id),
            relation: relation.to_string(),
            subject: SubjectRef::direct(subject_type, subject_id),
        }])
        .await
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn connect_to_server() {
    let (endpoint, handle) = start_server().await;
    let _client = setup_client(&endpoint).await;
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn check_permission_granted() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission can_view = viewer\n}",
    ).await;

    write_touch(&mut client, "document", "readme", "viewer", "user", "alice").await;

    let result = client
        .check_permission(CheckPermissionRequest {
            resource: ObjectRef::new("document", "readme"),
            permission: "can_view".to_string(),
            subject: SubjectRef::direct("user", "alice"),
            consistency: Consistency::FullConsistency,
        })
        .await
        .unwrap();

    assert!(result.allowed);
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn check_permission_denied() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission can_view = viewer\n}",
    ).await;

    write_touch(&mut client, "document", "readme", "viewer", "user", "alice").await;

    let result = client
        .check_permission(CheckPermissionRequest {
            resource: ObjectRef::new("document", "readme"),
            permission: "can_view".to_string(),
            subject: SubjectRef::direct("user", "bob"),
            consistency: Consistency::FullConsistency,
        })
        .await
        .unwrap();

    assert!(!result.allowed);
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn write_and_read_relationships() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission can_view = viewer\n}",
    ).await;

    write_touch(&mut client, "document", "readme", "viewer", "user", "alice").await;
    write_touch(&mut client, "document", "readme", "viewer", "user", "bob").await;

    let relationships = client
        .read_relationships(ReadRelationshipsRequest {
            filter: TupleFilter {
                object_type: Some("document".to_string()),
                object_id: Some("readme".to_string()),
                relation: Some("viewer".to_string()),
                ..Default::default()
            },
            consistency: Consistency::FullConsistency,
            limit: None,
        })
        .await
        .unwrap();

    assert_eq!(relationships.len(), 2);
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn lookup_resources() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission can_view = viewer\n}",
    ).await;

    write_touch(&mut client, "document", "doc1", "viewer", "user", "alice").await;
    write_touch(&mut client, "document", "doc2", "viewer", "user", "alice").await;
    write_touch(&mut client, "document", "doc3", "viewer", "user", "bob").await;

    let mut resource_ids = client
        .lookup_resources(LookupResourcesRequest {
            resource_type: "document".to_string(),
            permission: "can_view".to_string(),
            subject: SubjectRef::direct("user", "alice"),
            consistency: Consistency::FullConsistency,
            limit: None,
        })
        .await
        .unwrap();

    resource_ids.sort();
    assert_eq!(resource_ids, vec!["doc1", "doc2"]);
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn lookup_subjects() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  relation owner: user\n  permission can_view = viewer + owner\n}",
    ).await;

    write_touch(&mut client, "document", "readme", "viewer", "user", "alice").await;
    write_touch(&mut client, "document", "readme", "owner", "user", "bob").await;

    let subjects = client
        .lookup_subjects(LookupSubjectsRequest {
            resource: ObjectRef::new("document", "readme"),
            permission: "can_view".to_string(),
            subject_type: "user".to_string(),
            consistency: Consistency::FullConsistency,
        })
        .await
        .unwrap();

    let mut subject_ids: Vec<_> = subjects.iter().map(|s| s.subject_id.as_str()).collect();
    subject_ids.sort();
    assert_eq!(subject_ids, vec!["alice", "bob"]);
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn schema_write_and_read() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    let schema_text = "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission can_view = viewer\n}";
    write_schema(&mut client, schema_text).await;

    let schema = client.read_schema().await.unwrap();
    let schema = schema.expect("schema should exist");

    assert!(schema.contains("definition user"));
    assert!(schema.contains("definition document"));
    assert!(schema.contains("relation viewer"));
    assert!(schema.contains("permission can_view"));
    handle.abort();
}

#[tokio::test]
#[ignore]
async fn computed_permission_via_union() {
    let (endpoint, handle) = start_server().await;
    let mut client = setup_client(&endpoint).await;

    write_schema(
        &mut client,
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  relation editor: user\n  permission can_edit = editor\n  permission can_view = viewer + can_edit\n}",
    ).await;

    write_touch(&mut client, "document", "readme", "editor", "user", "alice").await;

    let result = client
        .check_permission(CheckPermissionRequest {
            resource: ObjectRef::new("document", "readme"),
            permission: "can_view".to_string(),
            subject: SubjectRef::direct("user", "alice"),
            consistency: Consistency::FullConsistency,
        })
        .await
        .unwrap();

    assert!(result.allowed);
    handle.abort();
}
