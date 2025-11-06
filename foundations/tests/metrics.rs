use foundations::telemetry::metrics::{self, metrics, Counter};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};

#[metrics]
mod regular {
    pub fn requests() -> Counter;
    #[optional]
    pub fn optional() -> Counter;
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

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let metrics = metrics::collect(&settings).expect("metrics should be collectable");

    // Global prefix defaults to "undefined" if not initialized
    assert!(metrics.contains("\nundefined_regular_requests 1\n"));
    assert!(!metrics.contains("\nundefined_regular_optional"));
    assert!(metrics.contains("\nlibrary_calls 1\n"));
    assert!(!metrics.contains("\nlibrary_optional"));
}
