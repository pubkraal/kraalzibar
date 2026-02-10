mod handlers;
mod types;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::middleware;
use axum::response::Response;
use axum::routing::{get, post};
use kraalzibar_core::tuple::TenantId;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

const MAX_REQUEST_BODY_SIZE: usize = 4 * 1024 * 1024; // 4 MB

use crate::metrics::Metrics;
use crate::service::AuthzService;

pub struct AppState<F: StoreFactory> {
    pub service: Arc<AuthzService<F>>,
    pub tenant_id: TenantId,
    pub metrics: Arc<Metrics>,
}

impl<F: StoreFactory> Clone for AppState<F> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            tenant_id: self.tenant_id.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

fn method_name_from_path(path: &str) -> Option<&'static str> {
    match path {
        "/v1/permissions/check" => Some("check"),
        "/v1/permissions/expand" => Some("expand"),
        "/v1/permissions/resources" => Some("lookup_resources"),
        "/v1/permissions/subjects" => Some("lookup_subjects"),
        "/v1/relationships/write" => Some("write_relationships"),
        "/v1/relationships/read" => Some("read_relationships"),
        "/v1/schema" => None, // disambiguated by HTTP method in handler
        _ => None,
    }
}

async fn metrics_middleware<F: StoreFactory>(
    State(state): State<AppState<F>>,
    request: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Response {
    state.metrics.record_request();

    let path = request.uri().path().to_string();
    let http_method = request.method().clone();
    let start = std::time::Instant::now();

    let response = next.run(request).await;
    let elapsed = start.elapsed();

    if response.status().is_success() {
        state.metrics.record_success();
    } else {
        state.metrics.record_error();
    }

    // /v1/schema uses HTTP method to distinguish write (POST) vs read (GET)
    let method_name = if path == "/v1/schema" {
        match http_method {
            axum::http::Method::POST => Some("write_schema"),
            axum::http::Method::GET => Some("read_schema"),
            _ => None,
        }
    } else {
        method_name_from_path(&path)
    };

    if let Some(name) = method_name {
        state.metrics.record_method_request(name, elapsed);
    }

    response
}

pub fn create_router<F>(state: AppState<F>) -> Router
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    Router::new()
        .route("/v1/permissions/check", post(handlers::check_permission))
        .route("/v1/permissions/expand", post(handlers::expand_permission))
        .route(
            "/v1/permissions/resources",
            post(handlers::lookup_resources),
        )
        .route("/v1/permissions/subjects", post(handlers::lookup_subjects))
        .route(
            "/v1/relationships/write",
            post(handlers::write_relationships),
        )
        .route("/v1/relationships/read", post(handlers::read_relationships))
        .route("/v1/schema", post(handlers::write_schema))
        .route("/v1/schema", get(handlers::read_schema))
        .route("/v1/watch", get(handlers::watch))
        .route("/healthz", get(handlers::healthz))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_SIZE))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            metrics_middleware,
        ))
        .with_state(state)
}
