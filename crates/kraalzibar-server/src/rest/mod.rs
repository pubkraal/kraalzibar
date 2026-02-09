mod handlers;
mod types;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::middleware;
use axum::response::Response;
use axum::routing::{get, post};
use kraalzibar_core::tuple::TenantId;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

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

async fn metrics_middleware<F: StoreFactory>(
    State(state): State<AppState<F>>,
    request: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Response {
    state.metrics.record_request();
    let response = next.run(request).await;
    if response.status().is_success() {
        state.metrics.record_success();
    } else {
        state.metrics.record_error();
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            metrics_middleware,
        ))
        .with_state(state)
}
