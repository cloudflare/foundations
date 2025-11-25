use foundations::telemetry::metrics::{self, metrics, Counter};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};

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
