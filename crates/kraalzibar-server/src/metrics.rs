use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

#[derive(Debug, Default)]
pub struct Metrics {
    request_total: AtomicU64,
    request_success: AtomicU64,
    request_error: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_request(&self) {
        self.request_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_success(&self) {
        self.request_success.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.request_error.fetch_add(1, Ordering::Relaxed);
    }

    pub fn request_total(&self) -> u64 {
        self.request_total.load(Ordering::Relaxed)
    }

    pub fn request_success(&self) -> u64 {
        self.request_success.load(Ordering::Relaxed)
    }

    pub fn request_error(&self) -> u64 {
        self.request_error.load(Ordering::Relaxed)
    }

    pub fn render_prometheus(&self) -> String {
        let mut output = String::new();
        output.push_str("# HELP kraalzibar_requests_total Total number of requests.\n");
        output.push_str("# TYPE kraalzibar_requests_total counter\n");
        output.push_str(&format!(
            "kraalzibar_requests_total {}\n",
            self.request_total()
        ));
        output.push_str("# HELP kraalzibar_requests_success_total Total successful requests.\n");
        output.push_str("# TYPE kraalzibar_requests_success_total counter\n");
        output.push_str(&format!(
            "kraalzibar_requests_success_total {}\n",
            self.request_success()
        ));
        output.push_str("# HELP kraalzibar_requests_error_total Total failed requests.\n");
        output.push_str("# TYPE kraalzibar_requests_error_total counter\n");
        output.push_str(&format!(
            "kraalzibar_requests_error_total {}\n",
            self.request_error()
        ));
        output
    }
}

pub async fn metrics_handler(State(metrics): State<Arc<Metrics>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        metrics.render_prometheus(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_are_zero() {
        let m = Metrics::new();
        assert_eq!(m.request_total(), 0);
        assert_eq!(m.request_success(), 0);
        assert_eq!(m.request_error(), 0);
    }

    #[test]
    fn record_increments_counters() {
        let m = Metrics::new();
        m.record_request();
        m.record_request();
        m.record_success();
        m.record_error();

        assert_eq!(m.request_total(), 2);
        assert_eq!(m.request_success(), 1);
        assert_eq!(m.request_error(), 1);
    }

    #[test]
    fn render_prometheus_format() {
        let m = Metrics::new();
        m.record_request();
        m.record_success();

        let output = m.render_prometheus();

        assert!(output.contains("# TYPE kraalzibar_requests_total counter"));
        assert!(output.contains("kraalzibar_requests_total 1"));
        assert!(output.contains("kraalzibar_requests_success_total 1"));
        assert!(output.contains("kraalzibar_requests_error_total 0"));
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let metrics = Arc::new(Metrics::new());
        metrics.record_request();
        metrics.record_request();
        metrics.record_success();

        let app = axum::Router::new()
            .route("/metrics", axum::routing::get(metrics_handler))
            .with_state(metrics);

        let server = axum_test::TestServer::new(app).unwrap();
        let response = server.get("/metrics").await;

        response.assert_status_ok();
        let body = response.text();
        assert!(body.contains("kraalzibar_requests_total 2"));
        assert!(body.contains("kraalzibar_requests_success_total 1"));
    }
}
