use std::fmt;
use std::time::Duration;

#[derive(Clone)]
pub struct ClientOptions {
    pub api_key: Option<String>,
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientOptions")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            api_key: None,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_options_defaults_are_sensible() {
        let opts = ClientOptions::default();

        assert!(opts.api_key.is_none());
        assert_eq!(opts.timeout, Duration::from_secs(30));
        assert_eq!(opts.connect_timeout, Duration::from_secs(5));
    }

    #[test]
    fn client_options_debug_redacts_api_key() {
        let opts = ClientOptions {
            api_key: Some("my-secret-key".to_string()),
            ..Default::default()
        };
        let debug_output = format!("{opts:?}");

        assert!(!debug_output.contains("my-secret-key"));
        assert!(debug_output.contains("[REDACTED]"));
    }
}
