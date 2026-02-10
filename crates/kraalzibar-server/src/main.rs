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
    let metrics = Arc::new(Metrics::new());
    let service = Arc::new(
        AuthzService::with_cache_config(
            Arc::clone(&factory),
            config.to_engine_config(),
            config.to_schema_limits(),
            config.cache.clone(),
        )
        .with_metrics(Arc::clone(&metrics)),
    );

    // TODO: Replace with tenant resolution from auth middleware
    let tenant_id = TenantId::new(uuid::Uuid::nil());

    let (mut health_reporter, health_service) = create_health_service();
    health_reporter
        .set_serving::<tonic_health::pb::health_server::HealthServer<tonic_health::server::HealthService>>()
        .await;

    let grpc_addr = config.grpc_addr().parse()?;
    let rest_addr: std::net::SocketAddr = config.rest_addr().parse()?;

    const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4 MB

    let permission_svc = PermissionServiceServer::new(PermissionServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ))
    .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
    let relationship_svc = RelationshipServiceServer::new(RelationshipServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ))
    .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
    let schema_svc = SchemaServiceServer::new(SchemaServiceImpl::new(
        Arc::clone(&service),
        tenant_id.clone(),
    ))
    .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);

    // REST server with metrics endpoint
    let rest_state = rest::AppState {
        service: Arc::clone(&service),
        tenant_id,
        metrics: Arc::clone(&metrics),
    };
    let rest_router = rest::create_router(rest_state).route(
        "/metrics",
        axum::routing::get(metrics::metrics_handler).with_state(Arc::clone(&metrics)),
    );

    tracing::info!(%grpc_addr, "gRPC server listening");
    tracing::info!(%rest_addr, "REST server listening");

    let (shutdown_tx, _) = tokio::sync::watch::channel(());
    let shutdown_rx_rest = shutdown_tx.subscribe();

    let grpc_server = tonic::transport::Server::builder()
        .add_service(health_service)
        .add_service(permission_svc)
        .add_service(relationship_svc)
        .add_service(schema_svc)
        .serve_with_shutdown(grpc_addr, shutdown_signal(shutdown_tx));

    let rest_listener = tokio::net::TcpListener::bind(rest_addr).await?;
    let rest_server = axum::serve(rest_listener, rest_router).with_graceful_shutdown(async move {
        let mut rx = shutdown_rx_rest;
        let _ = rx.changed().await;
    });

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

async fn shutdown_signal(shutdown_tx: tokio::sync::watch::Sender<()>) {
    let ctrl_c = tokio::signal::ctrl_c();

    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                _ = ctrl_c => { tracing::info!("received SIGINT"); }
                _ = sigterm.recv() => { tracing::info!("received SIGTERM"); }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to register SIGTERM handler, using SIGINT only");
            let _ = ctrl_c.await;
            tracing::info!("received SIGINT");
        }
    }

    let _ = shutdown_tx.send(());
}
