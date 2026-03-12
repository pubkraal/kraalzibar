use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;

use crate::config::RateLimitConfig;

pub struct AuthFailureLimiter {
    cache: Cache<String, u32>,
    threshold: u32,
}

impl AuthFailureLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        let cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(config.auth_failure_window_seconds))
            .build();

        Self {
            cache,
            threshold: config.auth_failure_threshold,
        }
    }

    pub fn is_blocked(&self, key_id: &str) -> bool {
        if self.threshold == 0 {
            return false;
        }

        self.cache.get(key_id).is_some_and(|c| c >= self.threshold)
    }

    pub fn record_failure(&self, key_id: &str) {
        if self.threshold == 0 {
            return;
        }

        let current = self.cache.get(key_id).unwrap_or(0);
        self.cache.insert(key_id.to_string(), current + 1);
    }

    pub fn record_success(&self, key_id: &str) {
        self.cache.invalidate(key_id);
    }
}

pub struct TenantRateLimiter {
    cache: Cache<String, u32>,
    max_per_second: u32,
}

impl TenantRateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        let cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(1))
            .build();

        Self {
            cache,
            max_per_second: config.max_requests_per_tenant_per_second,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_per_second > 0
    }

    pub fn check_rate_limit(&self, tenant_id: &str) -> bool {
        if !self.is_enabled() {
            return true;
        }

        let current = self.cache.get(tenant_id).unwrap_or(0);

        if current >= self.max_per_second {
            return false;
        }

        self.cache.insert(tenant_id.to_string(), current + 1);

        true
    }
}

#[derive(Clone)]
pub struct RateLimitState {
    pub auth_failure_limiter: Arc<AuthFailureLimiter>,
    pub tenant_rate_limiter: Arc<TenantRateLimiter>,
}

impl RateLimitState {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            auth_failure_limiter: Arc::new(AuthFailureLimiter::new(config)),
            tenant_rate_limiter: Arc::new(TenantRateLimiter::new(config)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> RateLimitConfig {
        RateLimitConfig::default()
    }

    fn config_with_threshold(threshold: u32) -> RateLimitConfig {
        RateLimitConfig {
            auth_failure_threshold: threshold,
            auth_failure_window_seconds: 60,
            ..Default::default()
        }
    }

    #[test]
    fn auth_failure_limiter_blocks_after_threshold() {
        let config = config_with_threshold(3);
        let limiter = AuthFailureLimiter::new(&config);

        assert!(!limiter.is_blocked("key1"));

        limiter.record_failure("key1");
        assert!(!limiter.is_blocked("key1"));

        limiter.record_failure("key1");
        assert!(!limiter.is_blocked("key1"));

        limiter.record_failure("key1");
        assert!(limiter.is_blocked("key1"));
    }

    #[test]
    fn auth_failure_limiter_independent_per_key() {
        let config = config_with_threshold(2);
        let limiter = AuthFailureLimiter::new(&config);

        limiter.record_failure("key1");
        limiter.record_failure("key1");
        assert!(limiter.is_blocked("key1"));
        assert!(!limiter.is_blocked("key2"));
    }

    #[test]
    fn auth_failure_limiter_success_clears_count() {
        let config = config_with_threshold(3);
        let limiter = AuthFailureLimiter::new(&config);

        limiter.record_failure("key1");
        limiter.record_failure("key1");
        limiter.record_success("key1");

        assert!(!limiter.is_blocked("key1"));
    }

    #[test]
    fn auth_failure_limiter_zero_threshold_never_blocks() {
        let config = config_with_threshold(0);
        let limiter = AuthFailureLimiter::new(&config);

        for _ in 0..100 {
            limiter.record_failure("key1");
        }

        assert!(!limiter.is_blocked("key1"));
    }

    #[test]
    fn tenant_rate_limiter_allows_within_limit() {
        let config = RateLimitConfig {
            max_requests_per_tenant_per_second: 5,
            ..Default::default()
        };
        let limiter = TenantRateLimiter::new(&config);

        for _ in 0..5 {
            assert!(limiter.check_rate_limit("tenant1"));
        }

        assert!(!limiter.check_rate_limit("tenant1"));
    }

    #[test]
    fn tenant_rate_limiter_independent_per_tenant() {
        let config = RateLimitConfig {
            max_requests_per_tenant_per_second: 2,
            ..Default::default()
        };
        let limiter = TenantRateLimiter::new(&config);

        assert!(limiter.check_rate_limit("tenant1"));
        assert!(limiter.check_rate_limit("tenant1"));
        assert!(!limiter.check_rate_limit("tenant1"));

        assert!(limiter.check_rate_limit("tenant2"));
    }

    #[test]
    fn tenant_rate_limiter_zero_means_unlimited() {
        let config = RateLimitConfig {
            max_requests_per_tenant_per_second: 0,
            ..Default::default()
        };
        let limiter = TenantRateLimiter::new(&config);

        assert!(!limiter.is_enabled());

        for _ in 0..1000 {
            assert!(limiter.check_rate_limit("tenant1"));
        }
    }

    #[test]
    fn rate_limit_state_creates_both_limiters() {
        let config = default_config();
        let state = RateLimitState::new(&config);

        assert!(!state.auth_failure_limiter.is_blocked("key1"));
        assert!(state.tenant_rate_limiter.is_enabled());
    }
}
