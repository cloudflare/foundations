use std::sync::Once;

use foundations_metrics::proto::{Counter, Gauge, LabelPair, Metric, MetricType};
use foundations_metrics::{EncodeMetric, MetricFamily, RegistrationMetadata};
use prometheus::core::Collector as _;
use prometheus::process_collector::ProcessCollector;
use prometheus::proto::{
    MetricFamily as PrometheusMetricFamily, MetricType as PrometheusMetricType,
};

static REGISTER: Once = Once::new();

/// Registers the Linux process metrics that the legacy backend exposed.
pub(super) fn register() {
    REGISTER.call_once(|| {
        foundations_metrics::register(
            Box::new(ProcessMetrics(ProcessCollector::for_self())) as Box<dyn EncodeMetric>,
            RegistrationMetadata::default()
                .unprefixed(true)
                .unlabeled(true),
        );
    });
}

/// Adapts the established process collector to Foundations' protobuf model.
struct ProcessMetrics(ProcessCollector);

impl EncodeMetric for ProcessMetrics {
    fn encode(&self) -> Vec<MetricFamily> {
        self.0
            .collect()
            .into_iter()
            .filter_map(convert_family)
            .collect()
    }
}

fn convert_family(family: PrometheusMetricFamily) -> Option<MetricFamily> {
    let metric_type = match family.get_field_type() {
        PrometheusMetricType::COUNTER => MetricType::Counter,
        PrometheusMetricType::GAUGE => MetricType::Gauge,
        _ => return None,
    };

    let metric = family
        .get_metric()
        .iter()
        .map(|metric| {
            let label = metric
                .get_label()
                .iter()
                .map(|label| LabelPair {
                    name: Some(label.name().to_owned()),
                    value: Some(label.value().to_owned()),
                })
                .collect();

            match metric_type {
                MetricType::Counter => Metric {
                    label,
                    counter: Some(Counter {
                        value: Some(metric.get_counter().value()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                MetricType::Gauge => Metric {
                    label,
                    gauge: Some(Gauge {
                        value: Some(metric.get_gauge().value()),
                    }),
                    ..Default::default()
                },
                _ => unreachable!("process metrics contain only counters and gauges"),
            }
        })
        .collect();

    Some(MetricFamily {
        name: Some(family.name().to_owned()),
        help: Some(family.help().to_owned()),
        r#type: Some(metric_type as i32),
        metric,
        unit: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_legacy_process_metric_families() {
        let families = ProcessMetrics(ProcessCollector::for_self()).encode();
        let expected = [
            ("process_cpu_seconds_total", MetricType::Counter),
            ("process_open_fds", MetricType::Gauge),
            ("process_max_fds", MetricType::Gauge),
            ("process_virtual_memory_bytes", MetricType::Gauge),
            ("process_resident_memory_bytes", MetricType::Gauge),
            ("process_start_time_seconds", MetricType::Gauge),
            ("process_threads", MetricType::Gauge),
        ];

        assert_eq!(families.len(), expected.len());

        for (name, metric_type) in expected {
            let family = families
                .iter()
                .find(|family| family.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing process metric family {name}"));

            assert_eq!(family.r#type, Some(metric_type as i32));
            assert_eq!(family.metric.len(), 1);
        }

        assert!(!foundations_metrics::encode_to_protobuf(&families).is_empty());
    }
}
