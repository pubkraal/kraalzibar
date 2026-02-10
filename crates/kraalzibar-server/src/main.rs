use std::sync::Arc;

use clap::Parser;
use kraalzibar_server::api_key_repository::ApiKeyRepository;
use kraalzibar_server::auth;
use kraalzibar_server::cli::{Cli, Command};
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
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[cfg(feature = "telemetry")]
use kraalzibar_server::telemetry;

fn init_logging(config: &AppConfig) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log.level));

    // OTel layer is typed to bare Registry, so it must be added first.
    // Layer order (bottom to top): Registry → OTel → EnvFilter → fmt
    let registry = tracing_subscriber::registry();

    #[cfg(feature = "telemetry")]
    let otel_provider = telemetry::init_telemetry(&config.tracing);

    #[cfg(feature = "telemetry")]
    let otel_layer = otel_provider.as_ref().map(telemetry::make_otel_layer);

    #[cfg(feature = "telemetry")]
    let registry = registry.with(otel_layer);

    let registry = registry.with(filter);

    match config.log.format {
        LogFormat::Json => {
            let fmt_layer = tracing_subscriber::fmt::layer().json();
            registry.with(fmt_layer).init();
        }
        LogFormat::Pretty => {
            let fmt_layer = tracing_subscriber::fmt::layer().pretty();
            registry.with(fmt_layer).init();
        }
    }

    #[cfg(feature = "telemetry")]
    if otel_provider.is_some() {
        tracing::info!("OpenTelemetry tracing enabled");
    }

    // Leak the provider into a static to keep it alive for the process lifetime.
    // Shutdown is handled by the runtime on process exit.
    #[cfg(feature = "telemetry")]
    if let Some(provider) = otel_provider {
        std::mem::forget(provider);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let config = AppConfig::load(cli.config.as_deref())?;
    init_logging(&config);

    match cli.command {
        Some(Command::Migrate) => run_migrate(&config).await,
        Some(Command::ProvisionTenant { name }) => run_provision_tenant(&config, &name).await,
        Some(Command::CreateApiKey { tenant_name }) => {
            run_create_api_key(&config, &tenant_name).await
        }
        Some(Command::Serve) | None => run_serve(config).await,
    }
}

async fn run_migrate(config: &AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("running database migrations");
    let pool = sqlx::PgPool::connect(&config.database.url).await?;
    kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool).await?;
    tracing::info!("migrations completed successfully");
    Ok(())
}

async fn run_provision_tenant(
    config: &AppConfig,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::PgPool::connect(&config.database.url).await?;
    kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool).await?;

    let tenant_id = TenantId::new(uuid::Uuid::new_v4());
    let factory = kraalzibar_storage::postgres::PostgresStoreFactory::new(pool);
    factory.provision_tenant(&tenant_id, name).await?;

    println!("Tenant provisioned successfully");
    println!("  Name:      {name}");
    println!("  Tenant ID: {}", tenant_id.as_uuid());
    Ok(())
}

async fn run_create_api_key(
    config: &AppConfig,
    tenant_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::PgPool::connect(&config.database.url).await?;
    kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool).await?;

    let row = sqlx::query_as::<_, (uuid::Uuid,)>("SELECT id FROM tenants WHERE name = $1")
        .bind(tenant_name)
        .fetch_optional(&pool)
        .await?;

    let tenant_uuid = match row {
        Some((id,)) => id,
        None => {
            eprintln!("Error: tenant '{tenant_name}' not found");
            std::process::exit(1);
        }
    };

    let (full_key, secret) = auth::generate_api_key();
    let (key_id, _) = auth::parse_api_key(&full_key).expect("generated key should parse");
    let key_hash = auth::hash_secret(&secret)?;

    let repo = ApiKeyRepository::new(pool);
    repo.insert(&TenantId::new(tenant_uuid), key_id, &key_hash)
        .await?;

    println!("API key created successfully");
    println!("  Tenant:  {tenant_name}");
    println!("  API Key: {full_key}");
    println!();
    println!("Store this key securely — it will not be shown again.");
    Ok(())
}

async fn run_serve(config: AppConfig) -> Result<(), Box<dyn std::error::Error>> {
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

    let default_tenant = TenantId::new(uuid::Uuid::nil());

    let (mut health_reporter, health_service) = create_health_service();
    health_reporter
        .set_serving::<tonic_health::pb::health_server::HealthServer<tonic_health::server::HealthService>>()
        .await;

    let grpc_addr = config.grpc_addr().parse()?;
    let rest_addr: std::net::SocketAddr = config.rest_addr().parse()?;

    const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4 MB

    let tenant_interceptor = {
        let tenant = default_tenant.clone();
        move |mut req: tonic::Request<()>| -> Result<tonic::Request<()>, tonic::Status> {
            req.extensions_mut().insert(tenant.clone());
            Ok(req)
        }
    };

    let permission_svc = tonic::service::interceptor::InterceptedService::new(
        PermissionServiceServer::new(
            PermissionServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        tenant_interceptor.clone(),
    );
    let relationship_svc = tonic::service::interceptor::InterceptedService::new(
        RelationshipServiceServer::new(
            RelationshipServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        tenant_interceptor.clone(),
    );
    let schema_svc = tonic::service::interceptor::InterceptedService::new(
        SchemaServiceServer::new(
            SchemaServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        tenant_interceptor,
    );

    // REST server with metrics endpoint
    let rest_state = rest::AppState {
        service: Arc::clone(&service),
        metrics: Arc::clone(&metrics),
    };
    let rest_router = rest::create_router(rest_state, default_tenant).route(
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
