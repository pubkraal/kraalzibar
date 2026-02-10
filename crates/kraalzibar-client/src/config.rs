use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub api_key: Option<String>,
    pub timeout: Duration,
    pub connect_timeout: Duration,
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
}
