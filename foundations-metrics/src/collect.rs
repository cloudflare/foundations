use foundations_metrics_registry::{iter, proto::LabelPair};

use crate::MetricFamily;
use crate::diagnostics::report_collect_error;
use crate::validation::{
    NAME_REQUIREMENT, ValidationContext, is_valid_name, sanitize_metric_family,
};

/// Options that control which registered metrics are collected and how the
/// service name is represented.
#[derive(Copy, Clone, Debug)]
pub struct CollectionOptions<'a> {
    /// Whether metrics registered as optional are included.
    pub include_optional: bool,

    /// Service name to add to collected metrics, if any.
    pub service_name: Option<&'a str>,

    /// How `service_name` is represented in collected metrics.
    pub service_name_format: ServiceNameFormat<'a>,
}

/// How a service name is represented in collected metrics.
#[derive(Copy, Clone, Debug)]
pub enum ServiceNameFormat<'a> {
    /// Prefix metric family names with the service name.
    MetricPrefix,

    /// Add the service name to every metric row under the given label name.
    LabelWithName(&'a str),
}

/// Collects the currently registered metrics into the canonical protobuf model.
pub fn collect(options: CollectionOptions) -> Vec<MetricFamily> {
    if options.service_name.is_some()
        && let ServiceNameFormat::LabelWithName(label_name) = options.service_name_format
        && !is_valid_name(label_name)
    {
        report_collect_error(format_args!(
            "non-fatal error while collecting metrics: invalid configured service label name {label_name:?}; expected {NAME_REQUIREMENT}; skipped all metric families"
        ));
        return Vec::new();
    }

    let mut collected = Vec::new();

    for registered in iter() {
        let metadata = registered.metadata();

        if metadata.optional && !options.include_optional {
            continue;
        }

        let mut families = registered.metric().encode();

        if let Some(service_name) = options.service_name {
            match options.service_name_format {
                ServiceNameFormat::MetricPrefix if !metadata.unprefixed => {
                    for family in &mut families {
                        if let Some(name) = &mut family.name {
                            name.insert(0, '_');
                            name.insert_str(0, service_name);
                        }
                    }
                }
                ServiceNameFormat::LabelWithName(label_name) => {
                    let service_label = LabelPair {
                        name: Some(label_name.to_owned()),
                        value: Some(service_name.to_owned()),
                    };

                    for family in &mut families {
                        let family_name = family.name.as_deref().unwrap_or_default();
                        family.metric.retain_mut(|metric| {
                            let mut has_same_value = false;
                            for label in &metric.label {
                                if label.name.as_deref() != Some(label_name) {
                                    continue;
                                }

                                if label.value.as_deref() != Some(service_name) {
                                    report_collect_error(format_args!(
                                        "non-fatal error while collecting metrics: skipped row in metric family {family_name:?}; service label {label_name:?} already has a different value"
                                    ));
                                    return false;
                                }
                                has_same_value = true;
                            }

                            if !has_same_value {
                                metric.label.insert(0, service_label.clone());
                            }
                            true
                        });
                    }
                }
                ServiceNameFormat::MetricPrefix => {}
            }
        }

        families.retain_mut(|family| sanitize_metric_family(family, ValidationContext::Collection));
        collected.extend(families);
    }

    collected
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{
        Bucket, Counter, Exemplar, Gauge, Histogram, LabelPair, Metric, MetricType,
    };

    use super::*;
    use crate::{EncodeMetric, RegistrationMetadata, register};

    struct TestMetric(&'static str);

    struct TestFamilyMetric(MetricFamily);

    impl EncodeMetric for TestMetric {
        fn encode(&self) -> Vec<MetricFamily> {
            vec![MetricFamily {
                name: Some(self.0.to_owned()),
                help: Some("Test metric.".to_owned()),
                r#type: Some(MetricType::Gauge as i32),
                metric: vec![Metric::default()],
                unit: None,
            }]
        }
    }

    impl EncodeMetric for TestFamilyMetric {
        fn encode(&self) -> Vec<MetricFamily> {
            vec![self.0.clone()]
        }
    }

    fn register_test_metric(name: &'static str, metadata: RegistrationMetadata) {
        register(
            Box::new(TestMetric(name)) as Box<dyn EncodeMetric>,
            metadata,
        );
    }

    fn register_test_family(family: MetricFamily) {
        register(
            Box::new(TestFamilyMetric(family)) as Box<dyn EncodeMetric>,
            RegistrationMetadata::default(),
        );
    }

    fn label(name: &str, value: &str) -> LabelPair {
        LabelPair {
            name: Some(name.to_owned()),
            value: Some(value.to_owned()),
        }
    }

    #[test]
    fn filters_optional_metrics_and_applies_service_prefix() {
        register_test_metric("collect_required_metric", RegistrationMetadata::default());
        register_test_metric(
            "collect_optional_metric",
            RegistrationMetadata::default().optional(true),
        );
        register_test_metric(
            "collect_unprefixed_metric",
            RegistrationMetadata::default().unprefixed(true),
        );

        let required = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::MetricPrefix,
        });
        let required_names: Vec<_> = required
            .iter()
            .filter_map(|family| family.name.as_deref())
            .collect();

        assert!(required_names.contains(&"test_service_collect_required_metric"));
        assert!(!required_names.contains(&"test_service_collect_optional_metric"));
        assert!(required_names.contains(&"collect_unprefixed_metric"));

        let with_optional = collect(CollectionOptions {
            include_optional: true,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::MetricPrefix,
        });

        assert!(with_optional.iter().any(|family| {
            family.name.as_deref() == Some("test_service_collect_optional_metric")
        }));
    }

    #[test]
    fn service_label_is_added_to_prefixed_and_unprefixed_metrics() {
        register_test_metric("collect_label_metric", RegistrationMetadata::default());
        register_test_metric(
            "collect_label_unprefixed_metric",
            RegistrationMetadata::default().unprefixed(true),
        );

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::LabelWithName("service"),
        });

        for name in ["collect_label_metric", "collect_label_unprefixed_metric"] {
            let family = families
                .iter()
                .find(|family| family.name.as_deref() == Some(name))
                .expect("registered metric should be collected");
            let label = &family.metric[0].label[0];

            assert_eq!(label.name.as_deref(), Some("service"));
            assert_eq!(label.value.as_deref(), Some("test_service"));
        }
    }

    #[test]
    fn keeps_nonstandard_service_prefixed_family_names() {
        register_test_metric(
            "collect_nonstandard_prefix_metric",
            RegistrationMetadata::default(),
        );

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("invalid-service"),
            service_name_format: ServiceNameFormat::MetricPrefix,
        });

        assert!(families.iter().any(|family| {
            family.name.as_deref() == Some("invalid-service_collect_nonstandard_prefix_metric")
        }));
    }

    #[test]
    fn keeps_nonstandard_service_label_names() {
        register_test_metric(
            "collect_nonstandard_service_label_metric",
            RegistrationMetadata::default(),
        );

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("test_service"),
            service_name_format: ServiceNameFormat::LabelWithName("service:name"),
        });

        let family = families
            .iter()
            .find(|family| {
                family.name.as_deref() == Some("collect_nonstandard_service_label_metric")
            })
            .expect("metric with a nonstandard service label name should remain");
        assert_eq!(
            family.metric[0].label[0],
            label("service:name", "test_service")
        );
    }

    #[test]
    fn service_label_insertion_is_idempotent_and_drops_different_values() {
        let service_label_name = "collect_service_collision_label";
        register_test_family(MetricFamily {
            name: Some("collect_service_collision_metric".to_owned()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![
                Metric {
                    label: vec![label("id", "same"), label(service_label_name, "wanted")],
                    gauge: Some(Gauge { value: Some(1.0) }),
                    ..Default::default()
                },
                Metric {
                    label: vec![label("id", "different"), label(service_label_name, "other")],
                    gauge: Some(Gauge { value: Some(2.0) }),
                    ..Default::default()
                },
                Metric {
                    label: vec![label("id", "absent")],
                    gauge: Some(Gauge { value: Some(3.0) }),
                    ..Default::default()
                },
            ],
            unit: None,
        });

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: Some("wanted"),
            service_name_format: ServiceNameFormat::LabelWithName(service_label_name),
        });
        let family = families
            .iter()
            .find(|family| family.name.as_deref() == Some("collect_service_collision_metric"))
            .expect("test family should be collected");

        assert_eq!(family.metric.len(), 2);
        let same = family
            .metric
            .iter()
            .find(|metric| metric.label[0].value.as_deref() == Some("same"))
            .expect("same-value row should remain");
        assert_eq!(
            same.label
                .iter()
                .filter(|label| label.name.as_deref() == Some(service_label_name))
                .count(),
            1
        );
        assert_eq!(same.label[0].name.as_deref(), Some("id"));

        let absent = family
            .metric
            .iter()
            .find(|metric| {
                metric
                    .label
                    .iter()
                    .any(|label| label.value.as_deref() == Some("absent"))
            })
            .expect("row without a service label should remain");
        assert_eq!(
            absent.label[0],
            label(service_label_name, "wanted"),
            "new service labels remain prepended"
        );
    }

    #[test]
    fn collection_keeps_nonstandard_names_and_skips_duplicate_and_reserved_labels() {
        register_test_family(MetricFamily {
            name: Some("collect_row_validation_gauge".to_owned()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![
                Metric {
                    label: vec![label("id", "valid")],
                    gauge: Some(Gauge { value: Some(1.0) }),
                    ..Default::default()
                },
                Metric {
                    label: vec![label("bad\nname", "nonstandard")],
                    gauge: Some(Gauge { value: Some(2.0) }),
                    ..Default::default()
                },
                Metric {
                    label: vec![label("dup", "a"), label("dup", "b")],
                    gauge: Some(Gauge { value: Some(3.0) }),
                    ..Default::default()
                },
            ],
            unit: None,
        });
        register_test_family(MetricFamily {
            name: Some("collect_row_validation_histogram".to_owned()),
            help: None,
            r#type: Some(MetricType::Histogram as i32),
            metric: vec![
                Metric {
                    histogram: Some(Histogram::default()),
                    ..Default::default()
                },
                Metric {
                    label: vec![label("le", "1")],
                    histogram: Some(Histogram::default()),
                    ..Default::default()
                },
            ],
            unit: None,
        });
        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: None,
            service_name_format: ServiceNameFormat::MetricPrefix,
        });

        for (name, expected_rows) in [
            ("collect_row_validation_gauge", 2),
            ("collect_row_validation_histogram", 1),
        ] {
            let family = families
                .iter()
                .find(|family| family.name.as_deref() == Some(name))
                .expect("valid family should remain");
            assert_eq!(family.metric.len(), expected_rows, "family {name}");
        }
    }

    #[test]
    fn collection_keeps_nonstandard_exemplar_names_and_drops_duplicates() {
        register_test_family(MetricFamily {
            name: Some("collect_counter_exemplar_validation".to_owned()),
            help: None,
            r#type: Some(MetricType::Counter as i32),
            metric: vec![Metric {
                counter: Some(Counter {
                    value: Some(1.0),
                    exemplar: Some(Exemplar {
                        label: vec![label("trace:id", "bad")],
                        value: Some(2.0),
                        timestamp: None,
                    }),
                    created_timestamp: None,
                }),
                ..Default::default()
            }],
            unit: None,
        });
        register_test_family(MetricFamily {
            name: Some("collect_histogram_exemplar_validation".to_owned()),
            help: None,
            r#type: Some(MetricType::Histogram as i32),
            metric: vec![Metric {
                histogram: Some(Histogram {
                    bucket: vec![Bucket {
                        exemplar: Some(Exemplar {
                            label: vec![label("dup", "a"), label("dup", "b")],
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    exemplars: vec![
                        Exemplar {
                            label: vec![label("bad name", "bad")],
                            timestamp: Some(Default::default()),
                            ..Default::default()
                        },
                        Exemplar {
                            label: vec![label("trace_id", "missing_timestamp")],
                            ..Default::default()
                        },
                        Exemplar {
                            label: vec![label("trace_id", "good")],
                            timestamp: Some(Default::default()),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            unit: None,
        });

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: None,
            service_name_format: ServiceNameFormat::MetricPrefix,
        });
        let counter = families
            .iter()
            .find(|family| family.name.as_deref() == Some("collect_counter_exemplar_validation"))
            .expect("counter family should remain");
        assert_eq!(
            counter.metric[0]
                .counter
                .as_ref()
                .unwrap()
                .exemplar
                .as_ref()
                .unwrap()
                .label[0]
                .name
                .as_deref(),
            Some("trace:id")
        );

        let histogram = families
            .iter()
            .find(|family| family.name.as_deref() == Some("collect_histogram_exemplar_validation"))
            .expect("histogram family should remain");
        let histogram = histogram.metric[0].histogram.as_ref().unwrap();
        assert!(histogram.bucket[0].exemplar.is_none());
        assert_eq!(histogram.exemplars.len(), 2);
        assert_eq!(
            histogram
                .exemplars
                .iter()
                .map(|exemplar| exemplar.label[0].name.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["bad name", "trace_id"]
        );
    }
}
