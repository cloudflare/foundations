#![cfg(feature = "foundations-metrics-backend")]

use foundations::telemetry::metrics::{HistogramBuilder, ThreadLocalHistogram, metrics};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};

mod common;
use common::collect_text;

#[metrics]
mod generated {
    /// Request duration.
    #[ctor = HistogramBuilder {
        buckets: &[1.0, 2.0],
    }]
    pub fn request_duration_seconds() -> ThreadLocalHistogram;
}

#[test]
fn a_thread_local_histogram_works_with_the_metrics_macro() {
    let histogram = generated::request_duration_seconds();
    histogram.observe(0.5);
    histogram.observe(1.5);

    let text = collect_text(&MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    });

    assert!(
        text.contains("undefined_generated_request_duration_seconds_count 2"),
        "collected output was: {text}"
    );
}
