use std::sync::Arc;

use clap::Parser;
use kraalzibar_server::api_key_repository::ApiKeyRepository;
use kraalzibar_server::auth;
use kraalzibar_server::cli::{Cli, Command};
use kraalzibar_server::config::{AppConfig, LogFormat};
use kraalzibar_server::grpc::{PermissionServiceImpl, RelationshipServiceImpl, SchemaServiceImpl};
use kraalzibar_server::health::create_health_service;
use kraalzibar_server::metrics::{self, Metrics};
use kraalzibar_server::middleware::AuthState;
use kraalzibar_server::proto::kraalzibar::v1::{
    permission_service_server::PermissionServiceServer,
    relationship_service_server::RelationshipServiceServer,
    schema_service_server::SchemaServiceServer,
};
use kraalzibar_server::rest;
use kraalzibar_server::service::AuthzService;
use kraalzibar_storage::InMemoryStoreFactory;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

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
        Some(Command::PurgeRelationships { tenant_name, yes }) => {
            run_purge_relationships(&config, &tenant_name, yes).await
        }
        Some(Command::Serve) | None => run_serve(config).await,
    }
}

async fn run_migrate(config: &AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    config.database.require_configured()?;
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
    config.database.require_configured()?;
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
    config.database.require_configured()?;
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

async fn run_purge_relationships(
    config: &AppConfig,
    tenant_name: &str,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    config.database.require_configured()?;
    let pool = sqlx::PgPool::connect(&config.database.url).await?;
    kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool).await?;

    let row = sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT id, pg_schema FROM tenants WHERE name = $1",
    )
    .bind(tenant_name)
    .fetch_optional(&pool)
    .await?;

    let (_tenant_uuid, pg_schema) = match row {
        Some(row) => row,
        None => {
            eprintln!("Error: tenant '{tenant_name}' not found");
            std::process::exit(1);
        }
    };

    let count_query = format!(
        "SELECT COUNT(*) FROM {pg_schema}.relation_tuples WHERE deleted_tx_id = 9223372036854775807"
    );
    let (count,): (i64,) = sqlx::query_as(&count_query).fetch_one(&pool).await?;

    if !yes {
        eprintln!(
            "WARNING: This will permanently delete all {count} relationship(s) for tenant '{tenant_name}'."
        );
        eprintln!("The schema definition will be preserved.");
        eprintln!();
        eprintln!("Re-run with --yes to confirm:");
        eprintln!("  kraalzibar-server purge-relationships --tenant-name {tenant_name} --yes");
        std::process::exit(1);
    }

    let delete_query = format!("DELETE FROM {pg_schema}.relation_tuples");
    let result = sqlx::query(&delete_query).execute(&pool).await?;

    let reset_seq = format!("ALTER SEQUENCE {pg_schema}.tx_id_seq RESTART WITH 1");
    sqlx::query(&reset_seq).execute(&pool).await?;

    println!(
        "Purged {} relationship(s) for tenant '{tenant_name}'. Transaction sequence reset.",
        result.rows_affected()
    );

    Ok(())
}

async fn run_serve(config: AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(
        grpc_addr = %config.grpc_addr(),
        rest_addr = %config.rest_addr(),
        "starting kraalzibar server"
    );

    if config.database.is_configured() {
        tracing::info!("database configured — using PostgreSQL backend with API key auth");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.database.max_connections)
            .min_connections(config.database.min_connections)
            .acquire_timeout(std::time::Duration::from_secs(
                config.database.acquire_timeout_seconds,
            ))
            .idle_timeout(std::time::Duration::from_secs(
                config.database.idle_timeout_seconds,
            ))
            .max_lifetime(std::time::Duration::from_secs(
                config.database.max_lifetime_seconds,
            ))
            .connect(&config.database.url)
            .await?;

        kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool).await?;

        let factory = Arc::new(kraalzibar_storage::postgres::PostgresStoreFactory::new(
            pool.clone(),
        ));
        let repo = Arc::new(ApiKeyRepository::new(pool));
        let auth_state = AuthState::with_repository(repo);

        start_server(config, factory, auth_state).await
    } else {
        tracing::warn!("no database URL configured — running in dev mode (in-memory, no auth)");
        let factory = Arc::new(InMemoryStoreFactory::new());
        let auth_state = AuthState::dev_mode();

        start_server(config, factory, auth_state).await
    }
}

async fn start_server<F>(
    config: AppConfig,
    factory: Arc<F>,
    auth_state: AuthState,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
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

    let (mut health_reporter, health_service) = create_health_service();
    health_reporter
        .set_serving::<tonic_health::pb::health_server::HealthServer<tonic_health::server::HealthService>>()
        .await;

    let grpc_addr = config.grpc_addr().parse()?;
    let rest_addr: std::net::SocketAddr = config.rest_addr().parse()?;

    const MAX_GRPC_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4 MB

    let grpc_auth = {
        let auth = auth_state.clone();
        move |req: tonic::Request<()>| -> Result<tonic::Request<()>, tonic::Status> {
            kraalzibar_server::middleware::grpc_auth_interceptor(&auth, req)
        }
    };

    let permission_svc = tonic::service::interceptor::InterceptedService::new(
        PermissionServiceServer::new(
            PermissionServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        grpc_auth.clone(),
    );
    let relationship_svc = tonic::service::interceptor::InterceptedService::new(
        RelationshipServiceServer::new(
            RelationshipServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        grpc_auth.clone(),
    );
    let schema_svc = tonic::service::interceptor::InterceptedService::new(
        SchemaServiceServer::new(
            SchemaServiceImpl::new(Arc::clone(&service)).with_metrics(Arc::clone(&metrics)),
        )
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        grpc_auth,
    );

    let rest_state = rest::AppState {
        service: Arc::clone(&service),
        metrics: Arc::clone(&metrics),
    };
    let rest_router = rest::create_router(rest_state, auth_state).route(
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
