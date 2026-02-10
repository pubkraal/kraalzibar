use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kraalzibar_core::tuple::TenantId;

use crate::api_key_repository::ApiKeyRepository;
use crate::auth;

#[derive(Clone)]
pub struct AuthState {
    repository: Option<Arc<ApiKeyRepository>>,
    dev_tenant: TenantId,
}

impl AuthState {
    pub fn dev_mode() -> Self {
        Self {
            repository: None,
            dev_tenant: TenantId::new(uuid::Uuid::nil()),
        }
    }

    pub fn with_repository(repository: Arc<ApiKeyRepository>) -> Self {
        Self {
            repository: Some(repository),
            dev_tenant: TenantId::new(uuid::Uuid::nil()),
        }
    }

    fn is_dev_mode(&self) -> bool {
        self.repository.is_none()
    }
}

fn skip_auth(path: &str) -> bool {
    matches!(path, "/healthz" | "/metrics")
}

pub async fn rest_auth_middleware(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    if skip_auth(&path) {
        return next.run(request).await;
    }

    if auth_state.is_dev_mode() {
        request
            .extensions_mut()
            .insert(auth_state.dev_tenant.clone());
        return next.run(request).await;
    }

    let repo = auth_state.repository.as_ref().unwrap();

    let auth_header = match request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        Some(h) => h.to_string(),
        None => {
            return error_json(StatusCode::UNAUTHORIZED, "missing authorization header");
        }
    };

    let raw_key = match auth_header.strip_prefix("Bearer ") {
        Some(k) => k,
        None => {
            return error_json(StatusCode::UNAUTHORIZED, "invalid authorization format");
        }
    };

    let (key_id, _) = match auth::parse_api_key(raw_key) {
        Ok(parts) => parts,
        Err(_) => {
            return error_json(StatusCode::UNAUTHORIZED, "invalid api key format");
        }
    };

    let lookup_result = match repo.lookup_by_key_id(key_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "api key lookup failed");
            return error_json(StatusCode::INTERNAL_SERVER_ERROR, "internal server error");
        }
    };

    match auth::authenticate(raw_key, |_| lookup_result) {
        Ok(ctx) => {
            request.extensions_mut().insert(ctx.tenant_id);
            next.run(request).await
        }
        Err(auth::AuthError::RevokedKey) => {
            error_json(StatusCode::UNAUTHORIZED, "api key has been revoked")
        }
        Err(_) => error_json(StatusCode::UNAUTHORIZED, "invalid api key"),
    }
}

#[allow(clippy::result_large_err)]
pub fn grpc_auth_interceptor(
    auth_state: &AuthState,
    mut request: tonic::Request<()>,
) -> Result<tonic::Request<()>, tonic::Status> {
    if auth_state.is_dev_mode() {
        request
            .extensions_mut()
            .insert(auth_state.dev_tenant.clone());
        return Ok(request);
    }

    let metadata = request.metadata();
    let auth_header = metadata
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| tonic::Status::unauthenticated("missing authorization header"))?;

    let raw_key = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| tonic::Status::unauthenticated("invalid authorization format"))?;

    let (key_id, _) = auth::parse_api_key(raw_key)
        .map_err(|_| tonic::Status::unauthenticated("invalid api key format"))?;

    let repo = auth_state.repository.as_ref().unwrap();

    // The interceptor is sync but the DB lookup is async. Use block_in_place
    // to bridge safely — this works on tokio's multi-threaded runtime (which
    // the server always uses). The lookup is a fast indexed query.
    let raw_key = raw_key.to_string();
    let lookup_result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(repo.lookup_by_key_id(key_id))
    })
    .map_err(|e| {
        tracing::error!(error = %e, "api key lookup failed");
        tonic::Status::internal("internal server error")
    })?;

    match auth::authenticate(&raw_key, |_| lookup_result) {
        Ok(ctx) => {
            request.extensions_mut().insert(ctx.tenant_id);
            Ok(request)
        }
        Err(auth::AuthError::RevokedKey) => {
            Err(tonic::Status::unauthenticated("api key has been revoked"))
        }
        Err(_) => Err(tonic::Status::unauthenticated("invalid api key")),
    }
}

fn error_json(status: StatusCode, msg: &str) -> Response {
    let body = serde_json::json!({"error": msg});
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::middleware;
    use axum::routing::get;
    use axum_test::TestServer;
    use serde_json::json;

    fn make_dev_server() -> TestServer {
        let auth = AuthState::dev_mode();
        let app = Router::new()
            .route(
                "/test",
                get(
                    |axum::Extension(tid): axum::Extension<TenantId>| async move {
                        axum::Json(json!({"tenant_id": tid.as_uuid().to_string()}))
                    },
                ),
            )
            .route(
                "/healthz",
                get(|| async { axum::Json(json!({"status": "ok"})) }),
            )
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                rest_auth_middleware,
            ))
            .with_state(auth);
        TestServer::new(app).unwrap()
    }

    #[tokio::test]
    async fn dev_mode_injects_nil_tenant() {
        let server = make_dev_server();
        let response = server.get("/test").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["tenant_id"], "00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn healthz_skips_auth() {
        let server = make_dev_server();
        let response = server.get("/healthz").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn missing_auth_header_returns_401() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let server = make_auth_server(auth);

        let response = server.get("/test").await;
        response.assert_status(StatusCode::UNAUTHORIZED);
        let body: serde_json::Value = response.json();
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("missing authorization")
        );
    }

    #[tokio::test]
    async fn invalid_key_format_returns_401() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let server = make_auth_server(auth);

        let response = server
            .get("/test")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static("Bearer not_a_valid_key"),
            )
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn db_error_returns_500() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let server = make_auth_server(auth);

        // When DB is unreachable, lookup fails and we return 500
        let response = server
            .get("/test")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static(
                    "Bearer kraalzibar_unknown1_12345678901234567890123456789012",
                ),
            )
            .await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn healthz_skips_auth_even_with_repo() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let server = make_auth_server(auth);

        let response = server.get("/healthz").await;
        response.assert_status_ok();
    }

    fn make_test_repo() -> Arc<ApiKeyRepository> {
        // Create a repo pointing to a non-existent DB. Tests that need real DB
        // lookups use testcontainers (ignored tests). For these unit tests, we
        // test the middleware flow — the repo call will fail but we handle that.
        //
        // Since we can't connect to a real DB in unit tests, we use a mock
        // approach: tests for "unknown key" work because the lookup returns an
        // error (connection refused) which maps to 500, but we specifically test
        // the auth header parsing flow.
        //
        // For the full happy-path test, see the integration tests.
        //
        // Actually, let's use a simulated approach: create a pool that will fail
        // on query (lazy connect). sqlx PgPool with invalid URL will only fail
        // on actual query execution, not on pool creation.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .unwrap();
        Arc::new(ApiKeyRepository::new(pool))
    }

    #[tokio::test]
    async fn grpc_dev_mode_injects_nil_tenant() {
        let auth = AuthState::dev_mode();
        let request = tonic::Request::new(());

        let result = grpc_auth_interceptor(&auth, request);
        assert!(result.is_ok());
        let req = result.unwrap();
        let tenant = req.extensions().get::<TenantId>().unwrap();
        assert_eq!(*tenant.as_uuid(), uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn grpc_missing_auth_header_returns_unauthenticated() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let request = tonic::Request::new(());

        let result = grpc_auth_interceptor(&auth, request);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn grpc_invalid_key_format_returns_unauthenticated() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let mut request = tonic::Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer not_valid".parse().unwrap());

        let result = grpc_auth_interceptor(&auth, request);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grpc_valid_format_key_attempts_db_lookup() {
        let repo = make_test_repo();
        let auth = AuthState::with_repository(repo);
        let mut request = tonic::Request::new(());
        request.metadata_mut().insert(
            "authorization",
            "Bearer kraalzibar_testkey1_12345678901234567890123456789012"
                .parse()
                .unwrap(),
        );

        let result = grpc_auth_interceptor(&auth, request);
        // With an unreachable DB, the lookup fails → internal error (not unauthenticated stub)
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(
            status.code(),
            tonic::Code::Internal,
            "expected Internal from DB error, got {:?}: {}",
            status.code(),
            status.message()
        );
    }

    fn make_auth_server(auth: AuthState) -> TestServer {
        let app = Router::new()
            .route(
                "/test",
                get(
                    |axum::Extension(tid): axum::Extension<TenantId>| async move {
                        axum::Json(json!({"tenant_id": tid.as_uuid().to_string()}))
                    },
                ),
            )
            .route(
                "/healthz",
                get(|| async { axum::Json(json!({"status": "ok"})) }),
            )
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                rest_auth_middleware,
            ))
            .with_state(auth);
        TestServer::new(app).unwrap()
    }
}
