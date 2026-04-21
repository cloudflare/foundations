use foundations::service_info;
use foundations::telemetry::metrics::{self, Counter, metrics};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat, TelemetrySettings};
use foundations::telemetry::{TelemetryConfig, TelemetryContext};

#[metrics]
mod regular {
    pub fn requests() -> Counter;
    #[optional]
    pub fn optional() -> Counter;
    #[cfg(foundations_unstable)]
    #[with_removal]
    pub fn dynamic(label: &'static str) -> Counter;
}

#[metrics(unprefixed)]
mod library {
    pub fn calls() -> Counter;
    #[optional]
    pub fn optional() -> Counter;
}

#[test]
fn metrics_unprefixed() {
    regular::requests().inc();
    regular::optional().inc();
    library::calls().inc();
    library::optional().inc();

    #[cfg(foundations_unstable)]
    {
        regular::dynamic("foo").inc();
        regular::dynamic("bar").inc();

        assert!(regular::dynamic_remove("foo"));
        assert!(!regular::dynamic_remove("baz"));
        regular::dynamic_clear();
    }

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let metrics = metrics::collect(&settings).expect("metrics should be collectable");

    // Global prefix defaults to "undefined" if not initialized
    assert!(metrics.contains("\nundefined_regular_requests 1\n"));
    assert!(!metrics.contains("\nundefined_regular_optional"));
    assert!(!metrics.contains("\nundefined_regular_dynamic"));
    assert!(metrics.contains("\nlibrary_calls 1\n"));
    assert!(!metrics.contains("\nlibrary_optional"));
}

#[tokio::test]
async fn test_context_cooperates_with_init() {
    let _ctx = TelemetryContext::test();

    let service_info = service_info!();
    let settings = TelemetrySettings::default();
    let config = TelemetryConfig {
        service_info: &service_info,
        settings: &settings,
        custom_server_routes: vec![],
    };
    foundations::telemetry::init(config)
        .expect("telemetry init should succeed under TestTelemetryContext");

    regular::requests().inc();
    library::calls().inc();

    let metrics = metrics::collect(&settings.metrics).expect("metrics should be collectable");

    // `TelemetryContext::test()` should initialize metrics registry with default service_info!()
    assert!(metrics.contains("\nfoundations_regular_requests 1\n"));
    assert!(metrics.contains("\nlibrary_calls 1\n"));
}
