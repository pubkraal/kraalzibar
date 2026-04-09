use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use kraalzibar_core::tuple::TenantId;

use crate::rate_limit::RateLimitState;

pub async fn rest_tenant_rate_limit_middleware(
    axum::extract::State(rate_limit): axum::extract::State<RateLimitState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !rate_limit.tenant_rate_limiter.is_enabled() {
        return next.run(request).await;
    }

    let tenant_id = request.extensions().get::<TenantId>();

    if let Some(tid) = tenant_id {
        let key = tid.as_uuid().to_string();

        if !rate_limit.tenant_rate_limiter.check_rate_limit(&key) {
            let body = serde_json::json!({"error": "rate limit exceeded"});

            return (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
        }
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::middleware;
    use axum::routing::get;
    use axum_test::TestServer;
    use serde_json::json;

    use crate::config::RateLimitConfig;
    use crate::middleware::AuthState;
    use crate::middleware::rest_auth_middleware;

    fn make_rate_limited_server(max_per_second: u32) -> TestServer {
        let rl_config = RateLimitConfig {
            max_requests_per_tenant_per_second: max_per_second,
            ..Default::default()
        };
        let rate_limit = RateLimitState::new(&rl_config);
        let auth = AuthState::dev_mode();

        let app = Router::new()
            .route(
                "/test",
                get(|| async { axum::Json(json!({"status": "ok"})) }),
            )
            .layer(middleware::from_fn_with_state(
                rate_limit.clone(),
                rest_tenant_rate_limit_middleware,
            ))
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                rest_auth_middleware,
            ))
            .with_state(auth);
        TestServer::new(app).unwrap()
    }

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let server = make_rate_limited_server(5);

        for _ in 0..5 {
            let response = server.get("/test").await;
            response.assert_status_ok();
        }
    }

    #[tokio::test]
    async fn blocks_requests_exceeding_limit() {
        let server = make_rate_limited_server(2);

        server.get("/test").await.assert_status_ok();
        server.get("/test").await.assert_status_ok();

        let response = server.get("/test").await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);

        let body: serde_json::Value = response.json();
        assert!(
            body["error"].as_str().unwrap().contains("rate limit"),
            "expected rate limit error, got: {body}"
        );
    }

    #[tokio::test]
    async fn unlimited_when_zero() {
        let server = make_rate_limited_server(0);

        for _ in 0..100 {
            let response = server.get("/test").await;
            response.assert_status_ok();
        }
    }
}
