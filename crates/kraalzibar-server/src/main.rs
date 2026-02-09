use std::path::PathBuf;
use std::sync::Arc;

use kraalzibar_server::config::{AppConfig, LogFormat};
use kraalzibar_server::grpc::{PermissionServiceImpl, RelationshipServiceImpl, SchemaServiceImpl};
use kraalzibar_server::health::create_health_service;
use kraalzibar_server::metrics::{self, Metrics};
use kraalzibar_server::proto::kraalzibar::v1::{
    permission_service_server::PermissionServiceServer,
    relationship_service_server::RelationshipServiceServer,
    schema_service_server::SchemaServiceServer,
};
use kraalzibar_server::rest;
use kraalzibar_server::service::AuthzService;
use kraalzibar_storage::InMemoryStoreFactory;

use kraalzibar_core::tuple::TenantId;
use tracing_subscriber::EnvFilter;

fn init_logging(config: &AppConfig) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log.level));

    match config.log.format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .init();
        }
        LogFormat::Pretty => {
            tracing_subscriber::fmt()
                .pretty()
                .with_env_filter(filter)
                .init();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args().nth(1).map(PathBuf::from);

    let config = AppConfig::load(config_path.as_deref())?;
    init_logging(&config);

    tracing::info!(
        grpc_addr = %config.grpc_addr(),
        rest_addr = %config.rest_addr(),
        "starting kraalzibar server"
    );

    let factory = Arc::new(InMemoryStoreFactory::new());
    let service = Arc::new(AuthzService::new(
        Arc::clone(&factory),
        config.to_engine_config(),
        config.to_schema_limits(),
    ));

    let metrics = Arc::new(Metrics::new());

    // TODO: Replace with tenant resolution from auth middleware
    let tenant_id = TenantId::new(uuid::Uuid::nil());

    let (mut health_reporter, health_service) = create_health_service();
    health_reporter
        .set_serving::<tonic_health::pb::health_server::HealthServer<tonic_health::server::HealthService>>()
        .await;

    let grpc_addr = config.grpc_addr().parse()?;
    let rest_addr: std::net::SocketAddr = config.rest_addr().parse()?;

    let permission_svc = PermissionServiceServer::new(PermissionServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ));
    let relationship_svc = RelationshipServiceServer::new(RelationshipServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ));
    let schema_svc = SchemaServiceServer::new(SchemaServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ));

    // REST server with metrics endpoint
    let rest_state = rest::AppState {
        service: Arc::clone(&service),
        tenant_id,
    };
    let rest_router = rest::create_router(rest_state).route(
        "/metrics",
        axum::routing::get(metrics::metrics_handler).with_state(Arc::clone(&metrics)),
    );

    tracing::info!(%grpc_addr, "gRPC server listening");
    tracing::info!(%rest_addr, "REST server listening");

    let grpc_server = tonic::transport::Server::builder()
        .add_service(health_service)
        .add_service(permission_svc)
        .add_service(relationship_svc)
        .add_service(schema_svc)
        .serve_with_shutdown(grpc_addr, shutdown_signal());

    let rest_listener = tokio::net::TcpListener::bind(rest_addr).await?;
    let rest_server = axum::serve(rest_listener, rest_router);

    tokio::select! {
        result = grpc_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "gRPC server error");
            }
        }
        result = rest_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "REST server error");
            }
        }
    }

    tracing::info!("server shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => { tracing::info!("received SIGINT"); }
        _ = sigterm.recv() => { tracing::info!("received SIGTERM"); }
    }
}
