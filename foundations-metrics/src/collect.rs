use foundations_metrics_registry::{iter, proto::LabelPair};

use crate::MetricFamily;

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
                        for metric in &mut family.metric {
                            metric.label.insert(0, service_label.clone());
                        }
                    }
                }
                ServiceNameFormat::MetricPrefix => {}
            }
        }

        collected.extend(families);
    }

    collected
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{Metric, MetricType};

    use super::*;
    use crate::{EncodeMetric, RegistrationMetadata, register};

    struct TestMetric(&'static str);

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

    fn register_test_metric(name: &'static str, metadata: RegistrationMetadata) {
        register(
            Box::new(TestMetric(name)) as Box<dyn EncodeMetric>,
            metadata,
        );
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
}
