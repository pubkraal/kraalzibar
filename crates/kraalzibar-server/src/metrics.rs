use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub const BUCKET_BOUNDARIES: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub struct Histogram {
    buckets: Vec<AtomicU64>,
    sum_bits: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    pub fn new() -> Self {
        let buckets = (0..BUCKET_BOUNDARIES.len())
            .map(|_| AtomicU64::new(0))
            .collect();
        Self {
            buckets,
            sum_bits: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, value: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);

        loop {
            let old_bits = self.sum_bits.load(Ordering::Relaxed);
            let old_sum = f64::from_bits(old_bits);
            let new_sum = old_sum + value;
            let new_bits = new_sum.to_bits();
            if self
                .sum_bits
                .compare_exchange_weak(old_bits, new_bits, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }

        for (i, &boundary) in BUCKET_BOUNDARIES.iter().enumerate() {
            if value <= boundary {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn sum_as_f64(&self) -> f64 {
        f64::from_bits(self.sum_bits.load(Ordering::Relaxed))
    }

    pub fn bucket_count(&self, index: usize) -> u64 {
        self.buckets[index].load(Ordering::Relaxed)
    }

    pub fn render(&self, name: &str, method: &str) -> String {
        let mut output = String::new();
        for (i, &boundary) in BUCKET_BOUNDARIES.iter().enumerate() {
            output.push_str(&format!(
                "{name}_bucket{{method=\"{method}\",le=\"{boundary}\"}} {}\n",
                self.bucket_count(i)
            ));
        }
        output.push_str(&format!(
            "{name}_bucket{{method=\"{method}\",le=\"+Inf\"}} {}\n",
            self.count()
        ));
        output.push_str(&format!(
            "{name}_sum{{method=\"{method}\"}} {}\n",
            self.sum_as_f64()
        ));
        output.push_str(&format!(
            "{name}_count{{method=\"{method}\"}} {}\n",
            self.count()
        ));
        output
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Histogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Histogram")
            .field("count", &self.count())
            .field("sum", &self.sum_as_f64())
            .finish()
    }
}

pub struct MethodMetrics {
    requests: AtomicU64,
    duration: Histogram,
}

impl MethodMetrics {
    pub fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            duration: Histogram::new(),
        }
    }

    pub fn record(&self, elapsed: Duration) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.duration.observe(elapsed.as_secs_f64());
    }

    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn duration_histogram(&self) -> &Histogram {
        &self.duration
    }
}

impl Default for MethodMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MethodMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodMetrics")
            .field("requests", &self.request_count())
            .field("duration", &self.duration)
            .finish()
    }
}

#[derive(Debug)]
pub struct Metrics {
    request_total: AtomicU64,
    request_success: AtomicU64,
    request_error: AtomicU64,
    schema_cache_hits: AtomicU64,
    schema_cache_misses: AtomicU64,
    check_cache_hits: AtomicU64,
    check_cache_misses: AtomicU64,
    method_metrics: Mutex<HashMap<String, Arc<MethodMetrics>>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            request_total: AtomicU64::new(0),
            request_success: AtomicU64::new(0),
            request_error: AtomicU64::new(0),
            schema_cache_hits: AtomicU64::new(0),
            schema_cache_misses: AtomicU64::new(0),
            check_cache_hits: AtomicU64::new(0),
            check_cache_misses: AtomicU64::new(0),
            method_metrics: Mutex::new(HashMap::new()),
        }
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_method_request(&self, method: &str, elapsed: Duration) {
        let mm = {
            let mut map = self.method_metrics.lock().unwrap();
            Arc::clone(
                map.entry(method.to_string())
                    .or_insert_with(|| Arc::new(MethodMetrics::new())),
            )
        };
        mm.record(elapsed);
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

    pub fn record_schema_cache_hit(&self) {
        self.schema_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_schema_cache_miss(&self) {
        self.schema_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_check_cache_hit(&self) {
        self.check_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_check_cache_miss(&self) {
        self.check_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn schema_cache_hits(&self) -> u64 {
        self.schema_cache_hits.load(Ordering::Relaxed)
    }

    pub fn schema_cache_misses(&self) -> u64 {
        self.schema_cache_misses.load(Ordering::Relaxed)
    }

    pub fn check_cache_hits(&self) -> u64 {
        self.check_cache_hits.load(Ordering::Relaxed)
    }

    pub fn check_cache_misses(&self) -> u64 {
        self.check_cache_misses.load(Ordering::Relaxed)
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
        output.push_str("# HELP kraalzibar_schema_cache_hits_total Schema cache hits.\n");
        output.push_str("# TYPE kraalzibar_schema_cache_hits_total counter\n");
        output.push_str(&format!(
            "kraalzibar_schema_cache_hits_total {}\n",
            self.schema_cache_hits()
        ));
        output.push_str("# HELP kraalzibar_schema_cache_misses_total Schema cache misses.\n");
        output.push_str("# TYPE kraalzibar_schema_cache_misses_total counter\n");
        output.push_str(&format!(
            "kraalzibar_schema_cache_misses_total {}\n",
            self.schema_cache_misses()
        ));
        output.push_str("# HELP kraalzibar_check_cache_hits_total Check cache hits.\n");
        output.push_str("# TYPE kraalzibar_check_cache_hits_total counter\n");
        output.push_str(&format!(
            "kraalzibar_check_cache_hits_total {}\n",
            self.check_cache_hits()
        ));
        output.push_str("# HELP kraalzibar_check_cache_misses_total Check cache misses.\n");
        output.push_str("# TYPE kraalzibar_check_cache_misses_total counter\n");
        output.push_str(&format!(
            "kraalzibar_check_cache_misses_total {}\n",
            self.check_cache_misses()
        ));

        let map = self.method_metrics.lock().unwrap();
        let mut methods: Vec<&String> = map.keys().collect();
        methods.sort();

        if !methods.is_empty() {
            output.push_str("# HELP kraalzibar_method_requests_total Total requests per method.\n");
            output.push_str("# TYPE kraalzibar_method_requests_total counter\n");
            for method in &methods {
                let mm = &map[*method];
                output.push_str(&format!(
                    "kraalzibar_method_requests_total{{method=\"{method}\"}} {}\n",
                    mm.request_count()
                ));
            }

            output.push_str(
                "# HELP kraalzibar_request_duration_seconds Request duration histogram.\n",
            );
            output.push_str("# TYPE kraalzibar_request_duration_seconds histogram\n");
            for method in &methods {
                let mm = &map[*method];
                output.push_str(
                    &mm.duration_histogram()
                        .render("kraalzibar_request_duration_seconds", method),
                );
            }
        }

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

    #[test]
    fn metrics_include_cache_counters() {
        let m = Metrics::new();
        assert_eq!(m.schema_cache_hits(), 0);
        assert_eq!(m.schema_cache_misses(), 0);
        assert_eq!(m.check_cache_hits(), 0);
        assert_eq!(m.check_cache_misses(), 0);
    }

    #[test]
    fn prometheus_output_includes_cache_metrics() {
        let m = Metrics::new();
        m.record_schema_cache_hit();
        m.record_check_cache_miss();

        let output = m.render_prometheus();
        assert!(
            output.contains("kraalzibar_schema_cache_hits_total 1"),
            "missing schema cache hits: {output}"
        );
        assert!(
            output.contains("kraalzibar_check_cache_misses_total 1"),
            "missing check cache misses: {output}"
        );
    }

    #[test]
    fn histogram_new_starts_at_zero() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.sum_as_f64(), 0.0);
        for i in 0..BUCKET_BOUNDARIES.len() {
            assert_eq!(h.bucket_count(i), 0);
        }
    }

    #[test]
    fn histogram_observe_increments_correct_buckets() {
        let h = Histogram::new();
        h.observe(0.003); // should increment buckets >= 0.005 and 0.001
        // Buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
        // 0.003 > 0.001, so bucket[0] (le=0.001) should NOT be incremented
        // 0.003 <= 0.005, so bucket[1] (le=0.005) SHOULD be incremented
        // All buckets with boundary >= 0.005 should be incremented
        assert_eq!(h.bucket_count(0), 0); // le=0.001: 0.003 > 0.001
        assert_eq!(h.bucket_count(1), 1); // le=0.005: 0.003 <= 0.005
        assert_eq!(h.bucket_count(2), 1); // le=0.01
        assert_eq!(h.bucket_count(11), 1); // le=10.0
    }

    #[test]
    fn histogram_observe_updates_sum_and_count() {
        let h = Histogram::new();
        h.observe(0.5);
        h.observe(1.5);
        assert_eq!(h.count(), 2);
        let sum = h.sum_as_f64();
        assert!((sum - 2.0).abs() < 0.001, "sum should be ~2.0, got {sum}");
    }

    #[test]
    fn histogram_render_produces_correct_prometheus_format() {
        let h = Histogram::new();
        h.observe(0.5);

        let output = h.render("kraalzibar_test_duration_seconds", "check");
        assert!(
            output.contains(
                r#"kraalzibar_test_duration_seconds_bucket{method="check",le="0.001"} 0"#
            ),
            "output: {output}"
        );
        assert!(
            output
                .contains(r#"kraalzibar_test_duration_seconds_bucket{method="check",le="0.5"} 1"#),
            "output: {output}"
        );
        assert!(
            output
                .contains(r#"kraalzibar_test_duration_seconds_bucket{method="check",le="+Inf"} 1"#),
            "output: {output}"
        );
        assert!(
            output.contains(r#"kraalzibar_test_duration_seconds_sum{method="check"}"#),
            "output: {output}"
        );
        assert!(
            output.contains(r#"kraalzibar_test_duration_seconds_count{method="check"} 1"#),
            "output: {output}"
        );
    }

    #[test]
    fn histogram_multiple_observations_accumulate() {
        let h = Histogram::new();
        h.observe(0.01);
        h.observe(0.02);
        h.observe(0.03);

        assert_eq!(h.count(), 3);
        // le=0.01: only first observation
        assert_eq!(h.bucket_count(2), 1);
        // le=0.025: first two observations
        assert_eq!(h.bucket_count(3), 2);
        // le=0.05: all three
        assert_eq!(h.bucket_count(4), 3);
    }

    #[test]
    fn histogram_inf_bucket_always_equals_count() {
        let h = Histogram::new();
        h.observe(0.001);
        h.observe(100.0); // larger than any bucket boundary
        h.observe(0.5);

        assert_eq!(h.count(), 3);
        // The last bucket (le=10.0) should be 2 (0.001 and 0.5, not 100.0)
        assert_eq!(h.bucket_count(BUCKET_BOUNDARIES.len() - 1), 2);
        // +Inf should be 3 (all observations) — that's the count
    }

    #[test]
    fn histogram_observe_exact_boundary_is_included() {
        let h = Histogram::new();
        // Observe a value exactly equal to a bucket boundary (0.01)
        h.observe(0.01);
        // Buckets: [0.001, 0.005, 0.01, ...]
        assert_eq!(h.bucket_count(0), 0); // le=0.001: 0.01 > 0.001
        assert_eq!(h.bucket_count(1), 0); // le=0.005: 0.01 > 0.005
        assert_eq!(h.bucket_count(2), 1); // le=0.01: 0.01 <= 0.01 (exact match)
        assert_eq!(h.bucket_count(3), 1); // le=0.025: 0.01 <= 0.025
    }

    #[test]
    fn method_metrics_new_starts_at_zero() {
        let mm = MethodMetrics::new();
        assert_eq!(mm.request_count(), 0);
        assert_eq!(mm.duration_histogram().count(), 0);
    }

    #[test]
    fn method_metrics_record_increments_counter_and_histogram() {
        let mm = MethodMetrics::new();
        mm.record(std::time::Duration::from_millis(50));

        assert_eq!(mm.request_count(), 1);
        assert_eq!(mm.duration_histogram().count(), 1);
    }

    #[test]
    fn metrics_render_includes_per_method_counters() {
        let m = Metrics::new();
        m.record_method_request("check", std::time::Duration::from_millis(5));

        let output = m.render_prometheus();
        assert!(
            output.contains(r#"kraalzibar_method_requests_total{method="check"} 1"#),
            "output: {output}"
        );
    }

    #[test]
    fn metrics_render_includes_histograms() {
        let m = Metrics::new();
        m.record_method_request("check", std::time::Duration::from_millis(5));

        let output = m.render_prometheus();
        assert!(
            output.contains(r#"kraalzibar_request_duration_seconds_bucket{method="check""#),
            "output: {output}"
        );
        assert!(
            output.contains(r#"kraalzibar_request_duration_seconds_count{method="check"} 1"#),
            "output: {output}"
        );
    }

    #[test]
    fn existing_global_counters_still_work() {
        let m = Metrics::new();
        m.record_request();
        m.record_success();
        m.record_error();
        m.record_schema_cache_hit();
        m.record_check_cache_miss();

        let output = m.render_prometheus();
        assert!(output.contains("kraalzibar_requests_total 1"));
        assert!(output.contains("kraalzibar_requests_success_total 1"));
        assert!(output.contains("kraalzibar_requests_error_total 1"));
        assert!(output.contains("kraalzibar_schema_cache_hits_total 1"));
        assert!(output.contains("kraalzibar_check_cache_misses_total 1"));
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
