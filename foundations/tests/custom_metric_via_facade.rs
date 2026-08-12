//! Verifies that a custom metric can be defined and registered through the
//! `foundations` facade alone, without depending on `foundations-metrics`.
#![cfg(feature = "foundations-metrics-backend")]

use std::sync::atomic::{AtomicU64, Ordering};

use foundations::telemetry::metrics::proto::{Gauge, Metric, MetricType};
use foundations::telemetry::metrics::{
    EncodeMetric, EncodeMetricValue, Family, MetricFamily, NamedMetric, RegistrationMetadata,
    register,
};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};
use serde::Serialize;

mod common;
use common::collect_text;

#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
struct Labels {
    mount: &'static str,
}

#[derive(Default)]
struct OpenFiles(AtomicU64);

impl EncodeMetricValue for OpenFiles {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        vec![MetricFamily {
            name: Some(String::new()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![Metric {
                gauge: Some(Gauge {
                    value: Some(self.0.load(Ordering::Relaxed) as f64),
                }),
                ..Default::default()
            }],
            unit: None,
        }]
    }
}

#[test]
fn a_custom_metric_is_exposed_through_the_facade() {
    let open_files = Family::<Labels, OpenFiles>::default();
    open_files
        .get_or_create(&Labels { mount: "/data" })
        .0
        .store(5, Ordering::Relaxed);

    register(
        Box::new(NamedMetric::new(
            "facade_open_files",
            "Number of open file descriptors.",
            open_files,
        )) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default(),
    );

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let text = collect_text(&settings);

    // Telemetry is never initialised here, so the service name prefix is the
    // `UNINITIALISED_SERVICE_NAME` sentinel from `telemetry::metrics::init`
    // ("undefined"). If that sentinel changes, this expectation changes with it.
    assert!(
        text.contains("undefined_facade_open_files{mount=\"/data\"} 5"),
        "collected output was: {text}"
    );
}
