mod handlers;
mod types;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use kraalzibar_core::tuple::TenantId;
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::service::AuthzService;

pub struct AppState<F: StoreFactory> {
    pub service: Arc<AuthzService<F>>,
    pub tenant_id: TenantId,
}

impl<F: StoreFactory> Clone for AppState<F> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            tenant_id: self.tenant_id.clone(),
        }
    }
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
        .with_state(state)
}
