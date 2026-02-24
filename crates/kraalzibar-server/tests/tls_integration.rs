use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_stream::StreamExt;
use tonic::transport::{Channel, ClientTlsConfig, Server};

fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

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
use kraalzibar_server::tls::{TlsConfigOptions, build_tls_server_config};
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

struct TestCert {
    cert_path: String,
    key_path: String,
    ca_pem: Vec<u8>,
}

fn generate_test_cert() -> TestCert {
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));

    let cert = params.self_signed(&key_pair).unwrap();
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");

    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, &key_pem).unwrap();

    let ca_pem = cert_pem.into_bytes();

    std::mem::forget(dir);

    TestCert {
        cert_path: cert_path.to_string_lossy().to_string(),
        key_path: key_path.to_string_lossy().to_string(),
        ca_pem,
    }
}

async fn start_tls_test_server(
    test_cert: &TestCert,
) -> (String, u16, tokio::sync::oneshot::Sender<()>) {
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

    let tls_config = build_tls_server_config(TlsConfigOptions {
        cert_path: &test_cert.cert_path,
        key_path: &test_cert.key_path,
        alpn_protocols: vec![b"h2".to_vec()],
    })
    .unwrap();

    let tls_acceptor = TlsAcceptor::from(tls_config);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let addr_str = format!("https://localhost:{port}");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let tcp_stream = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let incoming = tcp_stream
        .then(move |conn| {
            let acceptor = tls_acceptor.clone();
            async move {
                match conn {
                    Ok(tcp) => acceptor.accept(tcp).await.ok().map(Ok::<_, std::io::Error>),
                    Err(_) => None,
                }
            }
        })
        .filter_map(|x| x);

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

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (addr_str, port, shutdown_tx)
}

fn full_consistency() -> Option<v1::Consistency> {
    Some(v1::Consistency {
        requirement: Some(v1::consistency::Requirement::FullConsistency(true)),
    })
}

#[tokio::test]
async fn grpc_tls_connection_succeeds() {
    ensure_crypto_provider();
    let test_cert = generate_test_cert();
    let (addr, _port, _shutdown) = start_tls_test_server(&test_cert).await;

    let ca_cert = tonic::transport::Certificate::from_pem(&test_cert.ca_pem);
    let tls_config = ClientTlsConfig::new()
        .domain_name("localhost")
        .ca_certificate(ca_cert);

    let channel = Channel::from_shared(addr)
        .unwrap()
        .tls_config(tls_config)
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut schema_client = SchemaServiceClient::new(channel.clone());
    let mut rel_client = RelationshipServiceClient::new(channel.clone());
    let mut perm_client = PermissionServiceClient::new(channel);

    schema_client
        .write_schema(v1::WriteSchemaRequest {
            schema: SCHEMA.to_string(),
            force: false,
        })
        .await
        .unwrap();

    rel_client
        .write_relationships(v1::WriteRelationshipsRequest {
            updates: vec![v1::RelationshipUpdate {
                operation: v1::relationship_update::Operation::Touch as i32,
                relationship: Some(v1::Relationship {
                    object: Some(v1::ObjectReference {
                        object_type: "document".to_string(),
                        object_id: "readme".to_string(),
                    }),
                    relation: "viewer".to_string(),
                    subject: Some(v1::SubjectReference {
                        subject_type: "user".to_string(),
                        subject_id: "alice".to_string(),
                        subject_relation: None,
                    }),
                }),
            }],
        })
        .await
        .unwrap();

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
}

#[tokio::test]
async fn plaintext_connection_rejected_when_tls_enabled() {
    ensure_crypto_provider();
    let test_cert = generate_test_cert();
    let (_, port, _shutdown) = start_tls_test_server(&test_cert).await;

    let plain_addr = format!("http://127.0.0.1:{port}");
    let result = Channel::from_shared(plain_addr).unwrap().connect().await;

    match result {
        Ok(channel) => {
            let mut client = SchemaServiceClient::new(channel);
            let rpc_result = client.read_schema(v1::ReadSchemaRequest {}).await;
            assert!(
                rpc_result.is_err(),
                "plaintext RPC should fail against TLS server"
            );
        }
        Err(_) => {
            // Connection refused or failed — also acceptable
        }
    }
}

#[tokio::test]
async fn rest_tls_connection_succeeds() {
    ensure_crypto_provider();
    let test_cert = generate_test_cert();

    let rest_tls = build_tls_server_config(TlsConfigOptions {
        cert_path: &test_cert.cert_path,
        key_path: &test_cert.key_path,
        alpn_protocols: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    })
    .unwrap();

    let factory = Arc::new(InMemoryStoreFactory::new());
    let service = Arc::new(AuthzService::new(
        factory,
        EngineConfig::default(),
        SchemaLimits::default(),
    ));
    let auth_state = AuthState::dev_mode();
    let metrics = Arc::new(kraalzibar_server::metrics::Metrics::new());

    let rest_state = kraalzibar_server::rest::AppState {
        service: Arc::clone(&service),
        metrics: Arc::clone(&metrics),
    };
    let rest_router = kraalzibar_server::rest::create_router(rest_state, auth_state);

    let rest_tls_config = axum_server::tls_rustls::RustlsConfig::from_config(rest_tls);
    let rest_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();

    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();

    let server = tokio::spawn(async move {
        axum_server::bind_rustls(rest_addr, rest_tls_config)
            .handle(server_handle)
            .serve(rest_router.into_make_service())
            .await
            .unwrap();
    });

    // Wait for server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let listening_addr = handle.listening().await.unwrap();

    // Build a reqwest client that trusts our self-signed cert
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&test_cert.ca_pem).unwrap())
        .build()
        .unwrap();

    let url = format!("https://localhost:{}/healthz", listening_addr.port());
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    handle.graceful_shutdown(Some(std::time::Duration::from_secs(1)));
    let _ = server.await;
}
