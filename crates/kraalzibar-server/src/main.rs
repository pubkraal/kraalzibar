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

    let pg_schema = match resolve_tenant_schema(&pool, tenant_name).await? {
        Some(schema) => schema,
        None => {
            eprintln!("Error: tenant '{tenant_name}' not found");
            std::process::exit(1);
        }
    };

    let count = count_active_relationships(&pool, &pg_schema).await?;

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

    let deleted = purge_relationships(&pool, &pg_schema).await?;

    println!(
        "Purged {deleted} relationship(s) for tenant '{tenant_name}'. Transaction sequence reset.",
    );

    Ok(())
}

async fn resolve_tenant_schema(
    pool: &sqlx::PgPool,
    tenant_name: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>("SELECT pg_schema FROM tenants WHERE name = $1")
        .bind(tenant_name)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|(schema,)| schema))
}

async fn count_active_relationships(
    pool: &sqlx::PgPool,
    pg_schema: &str,
) -> Result<i64, sqlx::Error> {
    let query = format!(
        "SELECT COUNT(*) FROM {pg_schema}.relation_tuples WHERE deleted_tx_id = 9223372036854775807"
    );
    let (count,): (i64,) = sqlx::query_as(&query).fetch_one(pool).await?;
    Ok(count)
}

async fn purge_relationships(pool: &sqlx::PgPool, pg_schema: &str) -> Result<u64, sqlx::Error> {
    let delete_query = format!("DELETE FROM {pg_schema}.relation_tuples");
    let result = sqlx::query(&delete_query).execute(pool).await?;

    let reset_seq = format!("ALTER SEQUENCE {pg_schema}.tx_id_seq RESTART WITH 1");
    sqlx::query(&reset_seq).execute(pool).await?;

    Ok(result.rows_affected())
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

#[cfg(test)]
mod purge_tests {
    use super::*;
    use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TupleWrite};
    use sqlx::PgPool;
    use std::sync::OnceLock;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn test_db_url() -> String {
        static URL: OnceLock<String> = OnceLock::new();

        if let Some(url) = URL.get() {
            return url.clone();
        }

        let container = Postgres::default()
            .with_tag("16-alpine")
            .start()
            .await
            .unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@localhost:{port}/postgres");

        let pool = PgPool::connect(&url).await.unwrap();
        kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool)
            .await
            .unwrap();
        pool.close().await;

        std::mem::forget(container);

        URL.get_or_init(|| url).clone()
    }

    async fn test_pool() -> PgPool {
        let url = test_db_url().await;
        PgPool::connect(&url).await.unwrap()
    }

    async fn provision_test_tenant(pool: &PgPool, name: &str) -> (TenantId, String) {
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        let factory = kraalzibar_storage::postgres::PostgresStoreFactory::new(pool.clone());
        factory.provision_tenant(&tenant_id, name).await.unwrap();

        let pg_schema = resolve_tenant_schema(pool, name).await.unwrap().unwrap();
        (tenant_id, pg_schema)
    }

    async fn write_tuples(pool: &PgPool, tenant_id: &TenantId, writes: &[TupleWrite]) {
        let factory = kraalzibar_storage::postgres::PostgresStoreFactory::new(pool.clone());
        let store = factory.for_tenant(tenant_id);
        store.write(writes, &[]).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn purge_deletes_all_tuples_for_target_tenant() {
        let pool = test_pool().await;
        let (tenant_id, pg_schema) = provision_test_tenant(&pool, "purge-all").await;

        write_tuples(
            &pool,
            &tenant_id,
            &[
                TupleWrite::new(
                    ObjectRef::new("doc", "readme"),
                    "viewer",
                    SubjectRef::direct("user", "alice"),
                ),
                TupleWrite::new(
                    ObjectRef::new("doc", "design"),
                    "editor",
                    SubjectRef::direct("user", "bob"),
                ),
            ],
        )
        .await;

        let count_before = count_active_relationships(&pool, &pg_schema).await.unwrap();
        assert_eq!(count_before, 2);

        let deleted = purge_relationships(&pool, &pg_schema).await.unwrap();
        assert_eq!(deleted, 2);

        let count_after = count_active_relationships(&pool, &pg_schema).await.unwrap();
        assert_eq!(count_after, 0);
    }

    #[tokio::test]
    #[ignore]
    async fn purge_resets_tx_sequence() {
        let pool = test_pool().await;
        let (tenant_id, pg_schema) = provision_test_tenant(&pool, "purge-seq").await;

        write_tuples(
            &pool,
            &tenant_id,
            &[TupleWrite::new(
                ObjectRef::new("doc", "readme"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            )],
        )
        .await;

        let query = format!("SELECT last_value FROM {pg_schema}.tx_id_seq WHERE is_called = true");
        let before: Option<(i64,)> = sqlx::query_as(&query).fetch_optional(&pool).await.unwrap();
        assert!(before.is_some(), "sequence should have been used");

        purge_relationships(&pool, &pg_schema).await.unwrap();

        let next_query = format!("SELECT nextval('{pg_schema}.tx_id_seq')");
        let (next_val,): (i64,) = sqlx::query_as(&next_query).fetch_one(&pool).await.unwrap();
        assert_eq!(next_val, 1, "next tx_id should restart from 1");
    }

    #[tokio::test]
    #[ignore]
    async fn purge_does_not_affect_other_tenants() {
        let pool = test_pool().await;
        let (tenant_a_id, pg_schema_a) = provision_test_tenant(&pool, "purge-tenant-a").await;
        let (tenant_b_id, pg_schema_b) = provision_test_tenant(&pool, "purge-tenant-b").await;

        write_tuples(
            &pool,
            &tenant_a_id,
            &[TupleWrite::new(
                ObjectRef::new("doc", "readme"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            )],
        )
        .await;

        write_tuples(
            &pool,
            &tenant_b_id,
            &[TupleWrite::new(
                ObjectRef::new("doc", "design"),
                "editor",
                SubjectRef::direct("user", "bob"),
            )],
        )
        .await;

        purge_relationships(&pool, &pg_schema_a).await.unwrap();

        let count_a = count_active_relationships(&pool, &pg_schema_a)
            .await
            .unwrap();
        assert_eq!(count_a, 0, "tenant A should have 0 tuples");

        let count_b = count_active_relationships(&pool, &pg_schema_b)
            .await
            .unwrap();
        assert_eq!(count_b, 1, "tenant B should still have 1 tuple");
    }

    #[tokio::test]
    #[ignore]
    async fn purge_nonexistent_tenant_returns_none() {
        let pool = test_pool().await;

        let result = resolve_tenant_schema(&pool, "no-such-tenant")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn purge_tenant_with_no_relationships_succeeds() {
        let pool = test_pool().await;
        let (_tenant_id, pg_schema) = provision_test_tenant(&pool, "purge-empty").await;

        let deleted = purge_relationships(&pool, &pg_schema).await.unwrap();
        assert_eq!(deleted, 0);

        let count = count_active_relationships(&pool, &pg_schema).await.unwrap();
        assert_eq!(count, 0);
    }
}
