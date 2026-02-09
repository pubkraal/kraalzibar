use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub grpc: GrpcConfig,
    pub rest: RestConfig,
    pub database: DatabaseConfig,
    pub engine: EngineConfigValues,
    pub schema_limits: SchemaLimitsConfig,
    pub log: LogConfig,
    pub cache: CacheConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RestConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

impl std::fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EngineConfigValues {
    pub max_depth: usize,
    pub max_concurrent_branches: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SchemaLimitsConfig {
    pub max_types: usize,
    pub max_relations_per_type: usize,
    pub max_permissions_per_type: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub format: LogFormat,
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub schema_cache_capacity: u64,
    pub schema_cache_ttl_seconds: u64,
    pub check_cache_capacity: u64,
    pub check_cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Json,
    Pretty,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 50051,
        }
    }
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgresql://localhost:5432/kraalzibar".to_string(),
            max_connections: 10,
        }
    }
}

impl Default for EngineConfigValues {
    fn default() -> Self {
        Self {
            max_depth: 6,
            max_concurrent_branches: 10,
        }
    }
}

impl Default for SchemaLimitsConfig {
    fn default() -> Self {
        Self {
            max_types: 50,
            max_relations_per_type: 30,
            max_permissions_per_type: 30,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            schema_cache_capacity: 1000,
            schema_cache_ttl_seconds: 30,
            check_cache_capacity: 10_000,
            check_cache_ttl_seconds: 300,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Json,
            level: "info".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load(config_path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut config = if let Some(path) = config_path {
            let contents = std::fs::read_to_string(path)
                .map_err(|e| ConfigError::ReadFile(path.display().to_string(), e.to_string()))?;
            toml::from_str::<AppConfig>(&contents)
                .map_err(|e| ConfigError::ParseToml(e.to_string()))?
        } else {
            AppConfig::default()
        };

        config.apply_env_overrides();
        config.validate()?;

        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        self.apply_env_overrides_with(|key| std::env::var(key).ok());
    }

    fn apply_env_overrides_with(&mut self, env: impl Fn(&str) -> Option<String>) {
        if let Some(v) = env("KRAALZIBAR_GRPC_HOST") {
            self.grpc.host = v;
        }
        if let Some(v) = env("KRAALZIBAR_GRPC_PORT")
            && let Ok(port) = v.parse()
        {
            self.grpc.port = port;
        }
        if let Some(v) = env("KRAALZIBAR_REST_HOST") {
            self.rest.host = v;
        }
        if let Some(v) = env("KRAALZIBAR_REST_PORT")
            && let Ok(port) = v.parse()
        {
            self.rest.port = port;
        }
        if let Some(v) = env("KRAALZIBAR_DATABASE_URL") {
            self.database.url = v;
        }
        if let Some(v) = env("KRAALZIBAR_DATABASE_MAX_CONNECTIONS")
            && let Ok(n) = v.parse()
        {
            self.database.max_connections = n;
        }
        if let Some(v) = env("KRAALZIBAR_ENGINE_MAX_DEPTH")
            && let Ok(n) = v.parse()
        {
            self.engine.max_depth = n;
        }
        if let Some(v) = env("KRAALZIBAR_LOG_LEVEL") {
            self.log.level = v;
        }
        if let Some(v) = env("KRAALZIBAR_LOG_FORMAT") {
            match v.as_str() {
                "json" => self.log.format = LogFormat::Json,
                "pretty" => self.log.format = LogFormat::Pretty,
                _ => {}
            }
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.grpc.port == 0 {
            return Err(ConfigError::Validation(
                "grpc.port must be non-zero".to_string(),
            ));
        }
        if self.rest.port == 0 {
            return Err(ConfigError::Validation(
                "rest.port must be non-zero".to_string(),
            ));
        }
        if self.engine.max_depth == 0 {
            return Err(ConfigError::Validation(
                "engine.max_depth must be non-zero".to_string(),
            ));
        }
        if self.database.max_connections == 0 {
            return Err(ConfigError::Validation(
                "database.max_connections must be non-zero".to_string(),
            ));
        }
        if self.engine.max_concurrent_branches == 0 {
            return Err(ConfigError::Validation(
                "engine.max_concurrent_branches must be non-zero".to_string(),
            ));
        }
        if self.schema_limits.max_types == 0 {
            return Err(ConfigError::Validation(
                "schema_limits.max_types must be non-zero".to_string(),
            ));
        }
        if self.schema_limits.max_relations_per_type == 0 {
            return Err(ConfigError::Validation(
                "schema_limits.max_relations_per_type must be non-zero".to_string(),
            ));
        }
        if self.schema_limits.max_permissions_per_type == 0 {
            return Err(ConfigError::Validation(
                "schema_limits.max_permissions_per_type must be non-zero".to_string(),
            ));
        }
        if self.cache.schema_cache_capacity == 0 {
            return Err(ConfigError::Validation(
                "cache.schema_cache_capacity must be non-zero".to_string(),
            ));
        }
        if self.cache.schema_cache_ttl_seconds == 0 {
            return Err(ConfigError::Validation(
                "cache.schema_cache_ttl_seconds must be non-zero".to_string(),
            ));
        }
        if self.cache.check_cache_capacity == 0 {
            return Err(ConfigError::Validation(
                "cache.check_cache_capacity must be non-zero".to_string(),
            ));
        }
        if self.cache.check_cache_ttl_seconds == 0 {
            return Err(ConfigError::Validation(
                "cache.check_cache_ttl_seconds must be non-zero".to_string(),
            ));
        }
        Ok(())
    }

    pub fn to_engine_config(&self) -> kraalzibar_core::engine::EngineConfig {
        kraalzibar_core::engine::EngineConfig {
            max_depth: self.engine.max_depth,
            max_concurrent_branches: self.engine.max_concurrent_branches,
        }
    }

    pub fn to_schema_limits(&self) -> kraalzibar_core::schema::SchemaLimits {
        kraalzibar_core::schema::SchemaLimits {
            max_types: self.schema_limits.max_types,
            max_relations_per_type: self.schema_limits.max_relations_per_type,
            max_permissions_per_type: self.schema_limits.max_permissions_per_type,
        }
    }

    pub fn grpc_addr(&self) -> String {
        format!("{}:{}", self.grpc.host, self.grpc.port)
    }

    pub fn rest_addr(&self) -> String {
        format!("{}:{}", self.rest.host, self.rest.port)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file '{0}': {1}")]
    ReadFile(String, String),

    #[error("failed to parse TOML config: {0}")]
    ParseToml(String),

    #[error("config validation failed: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_has_sensible_values() {
        let config = AppConfig::default();

        assert_eq!(config.grpc.host, "0.0.0.0");
        assert_eq!(config.grpc.port, 50051);
        assert_eq!(config.engine.max_depth, 6);
        assert_eq!(config.database.max_connections, 10);
        assert_eq!(config.log.format, LogFormat::Json);
    }

    #[test]
    fn load_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"
[grpc]
host = "127.0.0.1"
port = 9090

[engine]
max_depth = 10

[log]
format = "pretty"
level = "debug"
"#
        )
        .unwrap();

        let config = AppConfig::load(Some(&path)).unwrap();

        assert_eq!(config.grpc.host, "127.0.0.1");
        assert_eq!(config.grpc.port, 9090);
        assert_eq!(config.engine.max_depth, 10);
        assert_eq!(config.log.format, LogFormat::Pretty);
        assert_eq!(config.log.level, "debug");
    }

    #[test]
    fn env_vars_override_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"
[grpc]
port = 9090
"#
        )
        .unwrap();

        // SAFETY: test runs single-threaded for this env var
        unsafe { std::env::set_var("KRAALZIBAR_GRPC_PORT", "8080") };
        let config = AppConfig::load(Some(&path)).unwrap();
        unsafe { std::env::remove_var("KRAALZIBAR_GRPC_PORT") };

        assert_eq!(config.grpc.port, 8080);
    }

    #[test]
    fn validation_rejects_zero_port() {
        let mut config = AppConfig::default();
        config.grpc.port = 0;

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::Validation(ref msg)) if msg.contains("port")));
    }

    #[test]
    fn validation_rejects_zero_max_depth() {
        let mut config = AppConfig::default();
        config.engine.max_depth = 0;

        let result = config.validate();
        assert!(
            matches!(result, Err(ConfigError::Validation(ref msg)) if msg.contains("max_depth"))
        );
    }

    #[test]
    fn validation_rejects_zero_max_concurrent_branches() {
        let mut config = AppConfig::default();
        config.engine.max_concurrent_branches = 0;

        let result = config.validate();
        assert!(
            matches!(result, Err(ConfigError::Validation(ref msg)) if msg.contains("max_concurrent_branches"))
        );
    }

    #[test]
    fn validation_rejects_zero_schema_limits() {
        let mut config = AppConfig::default();
        config.schema_limits.max_types = 0;

        let result = config.validate();
        assert!(
            matches!(result, Err(ConfigError::Validation(ref msg)) if msg.contains("max_types"))
        );
    }

    #[test]
    fn cache_config_exposed_in_app_config() {
        let toml_str = r#"
[cache]
schema_cache_capacity = 500
schema_cache_ttl_seconds = 60
check_cache_capacity = 5000
check_cache_ttl_seconds = 120
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(config.cache.schema_cache_capacity, 500);
        assert_eq!(config.cache.schema_cache_ttl_seconds, 60);
        assert_eq!(config.cache.check_cache_capacity, 5000);
        assert_eq!(config.cache.check_cache_ttl_seconds, 120);
    }

    #[test]
    fn cache_config_validates_nonzero_capacity() {
        let mut config = AppConfig::default();
        config.cache.schema_cache_capacity = 0;

        let result = config.validate();
        assert!(
            matches!(result, Err(ConfigError::Validation(ref msg)) if msg.contains("schema_cache_capacity"))
        );
    }

    #[test]
    fn database_config_debug_redacts_url() {
        let config = DatabaseConfig {
            url: "postgresql://user:secret_password@host:5432/db".to_string(),
            max_connections: 10,
        };

        let debug_output = format!("{config:?}");

        assert!(
            !debug_output.contains("secret_password"),
            "debug output should not contain password: {debug_output}"
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "debug output should contain [REDACTED]: {debug_output}"
        );
    }

    #[test]
    fn env_override_tests_use_mock_reader() {
        let mut config = AppConfig::default();
        let env = |key: &str| -> Option<String> {
            match key {
                "KRAALZIBAR_GRPC_PORT" => Some("9999".to_string()),
                "KRAALZIBAR_REST_PORT" => Some("7777".to_string()),
                _ => None,
            }
        };
        config.apply_env_overrides_with(env);

        assert_eq!(config.grpc.port, 9999);
        assert_eq!(config.rest.port, 7777);
    }
}
