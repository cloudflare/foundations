//! [`clear_all`] changes process-wide collection state, so it's tested in its
//! own binary.

use foundations_metrics::proto::Metric;
use foundations_metrics::{
    CollectionOptions, Counter, EncodeMetric, Family, Gauge, InfoMetric, NamedMetric,
    RegistrationMetadata, ServiceNameFormat, clear_all, collect, register, report_info,
};
use serde::Serialize;

#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
struct Labels {
    status: u16,
}

#[derive(Serialize)]
struct BuildInfo {
    version: &'static str,
}

impl InfoMetric for BuildInfo {
    const NAME: &'static str = "build_info";
    const HELP: &'static str = "Build information.";
}

fn register_named<M>(metric: NamedMetric<M>)
where
    NamedMetric<M>: EncodeMetric,
{
    register(
        Box::new(metric) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default(),
    );
}

/// Formats each collected row as `name{labels} value`, sorted.
fn collected() -> Vec<String> {
    let families = collect(CollectionOptions {
        include_optional: false,
        service_name: None,
        service_name_format: ServiceNameFormat::MetricPrefix,
    });

    let mut rows: Vec<String> = families
        .iter()
        .flat_map(|family| {
            let name = family.name.as_deref().unwrap_or_default();
            family.metric.iter().map(move |row| format_row(name, row))
        })
        .collect();

    rows.sort();
    rows
}

fn format_row(name: &str, row: &Metric) -> String {
    let value = row
        .counter
        .as_ref()
        .and_then(|counter| counter.value)
        .or_else(|| row.gauge.as_ref().and_then(|gauge| gauge.value))
        .expect("row has a value");

    let labels: Vec<String> = row
        .label
        .iter()
        .map(|label| {
            format!(
                "{}=\"{}\"",
                label.name.as_deref().unwrap_or_default(),
                label.value.as_deref().unwrap_or_default()
            )
        })
        .collect();

    if labels.is_empty() {
        format!("{name} {value}")
    } else {
        format!("{name}{{{}}} {value}", labels.join(","))
    }
}

#[test]
fn reports_only_series_that_change_after_clear_all() {
    let requests = Family::<Labels, Counter>::default();
    requests.get_or_create(&Labels { status: 200 }).inc_by(553);
    requests.get_or_create(&Labels { status: 404 }).inc_by(12);
    register_named(NamedMetric::new(
        "requests_total",
        "Requests.",
        requests.clone(),
    ));

    let connections: Gauge = Gauge::default();
    connections.set(50);
    register_named(NamedMetric::new(
        "connections",
        "Open connections.",
        connections.clone(),
    ));

    report_info(BuildInfo { version: "1.2.3" });

    clear_all();

    // Info metrics are always reported.
    assert_eq!(collected(), ["build_info{version=\"1.2.3\"} 1"]);

    requests.get_or_create(&Labels { status: 200 }).inc();
    connections.dec();

    assert_eq!(
        collected(),
        [
            "build_info{version=\"1.2.3\"} 1",
            "connections 49",
            "requests_total{status=\"200\"} 554",
        ]
    );

    let later: Counter = Counter::default();
    register_named(NamedMetric::new("later_total", "Registered later.", later));

    assert_eq!(
        collected(),
        [
            "build_info{version=\"1.2.3\"} 1",
            "connections 49",
            "later_total 0",
            "requests_total{status=\"200\"} 554",
        ]
    );
}
