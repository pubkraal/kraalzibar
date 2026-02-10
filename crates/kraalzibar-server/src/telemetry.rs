use crate::config::TracingConfig;

#[cfg(feature = "telemetry")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "telemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::TracerProvider;

#[cfg(feature = "telemetry")]
pub fn init_telemetry(config: &TracingConfig) -> Option<TracerProvider> {
    if !config.enabled {
        return None;
    }

    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&config.otlp_endpoint);

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            opentelemetry_sdk::trace::Config::default()
                .with_sampler(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(
                    config.sample_rate,
                ))
                .with_resource(opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", config.service_name.clone()),
                ])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .expect("failed to install OpenTelemetry tracer");

    Some(provider)
}

#[cfg(feature = "telemetry")]
pub fn make_otel_layer(
    provider: &TracerProvider,
) -> tracing_opentelemetry::OpenTelemetryLayer<
    tracing_subscriber::Registry,
    opentelemetry_sdk::trace::Tracer,
> {
    let tracer = provider.tracer("kraalzibar");
    tracing_opentelemetry::layer().with_tracer(tracer)
}

#[cfg(feature = "telemetry")]
pub fn shutdown_telemetry(provider: TracerProvider) {
    provider
        .shutdown()
        .expect("failed to shut down tracer provider");
}

#[cfg(not(feature = "telemetry"))]
pub fn init_telemetry(_config: &TracingConfig) -> Option<()> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_telemetry_returns_none_when_disabled() {
        let config = TracingConfig {
            enabled: false,
            ..Default::default()
        };
        let result = init_telemetry(&config);
        assert!(result.is_none());
    }

    #[test]
    fn tracing_config_enabled_field_controls_init() {
        let disabled = TracingConfig {
            enabled: false,
            otlp_endpoint: "http://localhost:4317".to_string(),
            service_name: "test".to_string(),
            sample_rate: 1.0,
        };
        assert!(init_telemetry(&disabled).is_none());
    }
}
