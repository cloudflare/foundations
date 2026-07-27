//! Info metrics: gauges whose value is always `1`, carrying their data in labels.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::OnceLock;

use foundations_metrics_registry::proto::{Gauge, LabelPair, Metric, MetricType};
use foundations_metrics_registry::{EncodeMetric, MetricFamily, RegistrationMetadata, register};
use parking_lot::RwLock;
use serde::Serialize;

use crate::diagnostics::report_collect_error;
use crate::labels::to_label_pairs;

/// Describes an info metric.
///
/// Info metrics expose textual information through their label set, which
/// should not change often during the process lifetime. Common examples are an
/// application's version, revision control commit, and the version of a
/// compiler.
pub trait InfoMetric: Serialize + Send + Sync + 'static {
    /// The name of the info metric.
    const NAME: &'static str;

    /// The help message of the info metric.
    const HELP: &'static str;
}

/// A reported info metric, with its label set already serialized.
struct InfoEntry {
    name: &'static str,
    help: &'static str,
    labels: Vec<LabelPair>,
}

static INFO_METRICS: OnceLock<RwLock<HashMap<TypeId, InfoEntry>>> = OnceLock::new();

/// Returns the info metric store, registering the collector on first use.
fn info_metrics() -> &'static RwLock<HashMap<TypeId, InfoEntry>> {
    INFO_METRICS.get_or_init(|| {
        // A single collector is registered for every info metric: the registry
        // is append-only, so registering per report would emit a duplicate
        // family each time the same info metric is reported again.
        register(
            Box::new(InfoCollector) as Box<dyn EncodeMetric>,
            // Info metrics are exposed exactly as reported: they keep their bare
            // name (e.g. `build_info`) and carry only their own fields as labels.
            RegistrationMetadata::default()
                .unprefixed(true)
                .unlabeled(true),
        );

        RwLock::new(HashMap::new())
    })
}

/// Registers an info metric, i.e. a gauge metric whose value is always `1`.
///
/// Reporting the same info metric type again replaces the previously reported
/// value, so a metric is exposed at most once per type.
///
/// Label serialization happens here rather than at collection time; a label set
/// that cannot be serialized is dropped with a non-fatal diagnostic.
///
/// ```
/// use foundations_metrics::{InfoMetric, report_info};
/// use serde::Serialize;
///
/// /// Build information.
/// #[derive(Serialize)]
/// struct BuildInfo {
///     version: &'static str,
/// }
///
/// impl InfoMetric for BuildInfo {
///     const NAME: &'static str = "build_info";
///     const HELP: &'static str = "Build information.";
/// }
///
/// report_info(BuildInfo { version: "1.2.3" });
/// ```
pub fn report_info<M>(info_metric: impl Into<Box<M>>)
where
    M: InfoMetric,
{
    let info_metric = info_metric.into();

    match to_label_pairs(&*info_metric) {
        Ok(labels) => {
            info_metrics().write().insert(
                TypeId::of::<M>(),
                InfoEntry {
                    name: M::NAME,
                    help: M::HELP,
                    labels,
                },
            );
        }
        Err(error) => report_collect_error(format_args!(
            "non-fatal error while reporting info metric {:?}: label serialization failed: {error}",
            M::NAME
        )),
    }
}

/// Encodes every reported info metric as a gauge family valued `1`.
struct InfoCollector;

impl EncodeMetric for InfoCollector {
    fn encode(&self) -> Vec<MetricFamily> {
        // Read without initializing: the collector is only ever registered from
        // `info_metrics`, so the store already exists by the time this runs.
        let Some(info_metrics) = INFO_METRICS.get() else {
            return Vec::new();
        };

        info_metrics
            .read()
            .values()
            .map(|entry| MetricFamily {
                name: Some(entry.name.to_owned()),
                help: Some(entry.help.to_owned()),
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric {
                    label: entry.labels.clone(),
                    gauge: Some(Gauge { value: Some(1.0) }),
                    ..Default::default()
                }],
                unit: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::{CollectionOptions, ServiceNameFormat, collect};

    fn family<'a>(families: &'a [MetricFamily], name: &str) -> &'a MetricFamily {
        families
            .iter()
            .find(|family| family.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("family {name:?} should be encoded"))
    }

    #[derive(Serialize)]
    struct EncodedInfo {
        version: &'static str,
    }

    impl InfoMetric for EncodedInfo {
        const NAME: &'static str = "info_encoded";
        const HELP: &'static str = "Encoded information.";
    }

    #[test]
    fn encodes_info_metric_as_a_gauge_valued_one() {
        report_info(EncodedInfo { version: "1.2.3" });

        let families = InfoCollector.encode();
        let family = family(&families, "info_encoded");

        assert_eq!(family.help.as_deref(), Some("Encoded information."));
        assert_eq!(family.r#type, Some(MetricType::Gauge as i32));
        assert_eq!(family.metric.len(), 1);
        assert_eq!(
            family.metric[0]
                .gauge
                .as_ref()
                .and_then(|gauge| gauge.value),
            Some(1.0)
        );
        assert_eq!(family.metric[0].label[0].name.as_deref(), Some("version"));
        assert_eq!(family.metric[0].label[0].value.as_deref(), Some("1.2.3"));
    }

    #[derive(Serialize)]
    struct ReplacedInfo {
        revision: &'static str,
    }

    impl InfoMetric for ReplacedInfo {
        const NAME: &'static str = "info_replaced";
        const HELP: &'static str = "Replaced information.";
    }

    #[test]
    fn reporting_the_same_info_metric_replaces_the_previous_value() {
        report_info(ReplacedInfo { revision: "first" });
        report_info(ReplacedInfo { revision: "second" });

        let families = InfoCollector.encode();
        let matching = families
            .iter()
            .filter(|family| family.name.as_deref() == Some("info_replaced"))
            .count();

        assert_eq!(matching, 1, "re-reporting must replace, not duplicate");
        assert_eq!(
            family(&families, "info_replaced").metric[0].label[0]
                .value
                .as_deref(),
            Some("second")
        );
    }

    #[derive(Serialize)]
    struct CollectedInfo {
        version: &'static str,
    }

    impl InfoMetric for CollectedInfo {
        const NAME: &'static str = "info_collected";
        const HELP: &'static str = "Collected information.";
    }

    #[test]
    fn collection_keeps_info_metric_names_unprefixed() {
        report_info(CollectedInfo { version: "4.5.6" });

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::MetricPrefix,
        });

        assert!(
            families
                .iter()
                .any(|family| family.name.as_deref() == Some("info_collected")),
            "info metrics keep their bare name"
        );
        assert!(
            !families
                .iter()
                .any(|family| family.name.as_deref() == Some("test_service_info_collected")),
            "info metrics are never service-prefixed"
        );
    }

    #[derive(Serialize)]
    struct LabeledInfo {
        version: &'static str,
    }

    impl InfoMetric for LabeledInfo {
        const NAME: &'static str = "info_labeled";
        const HELP: &'static str = "Labeled information.";
    }

    // Adding the service label would change the series identity of every info
    // metric relative to the representation collectors already scrape.
    #[test]
    fn collection_does_not_add_the_service_label_to_info_metrics() {
        report_info(LabeledInfo { version: "7.8.9" });

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::LabelWithName("service"),
        });

        let family = family(&families, "info_labeled");
        let label_names: Vec<_> = family.metric[0]
            .label
            .iter()
            .map(|label| label.name.as_deref().unwrap_or_default())
            .collect();

        assert_eq!(label_names, ["version"]);
    }
}
