//! Defines and registers custom metrics from outside the crate, which is the
//! only way to verify that the public API is sufficient on its own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use foundations_metrics::proto::{Gauge, Metric, MetricType};
use foundations_metrics::{
    CollectionOptions, EncodeMetric, EncodeMetricValue, Family, MetricFamily, NamedMetric,
    RegistrationMetadata, ServiceNameFormat, collect, register, to_label_pairs,
};
use serde::Serialize;

#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
struct Labels {
    mount: &'static str,
}

fn gauge(value: u64) -> Option<Gauge> {
    Some(Gauge {
        value: Some(value as f64),
    })
}

fn collected() -> Vec<MetricFamily> {
    collect(CollectionOptions {
        include_optional: false,
        service_name: None,
        service_name_format: ServiceNameFormat::MetricPrefix,
    })
}

fn family_named<'a>(families: &'a [MetricFamily], name: &str) -> &'a MetricFamily {
    families
        .iter()
        .find(|family| family.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("family {name:?} should be collected"))
}

/// A metric without labels only needs [`EncodeMetric`], which supplies its own
/// absolute name.
#[derive(Default)]
struct OpenFiles(AtomicU64);

impl EncodeMetric for OpenFiles {
    fn encode(&self) -> Vec<MetricFamily> {
        vec![MetricFamily {
            name: Some("open_files".to_owned()),
            help: Some("Number of open file descriptors.".to_owned()),
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![Metric {
                gauge: gauge(self.0.load(Ordering::Relaxed)),
                ..Default::default()
            }],
            unit: None,
        }]
    }
}

#[test]
fn a_custom_metric_can_be_registered_and_collected() {
    let files = OpenFiles::default();
    files.0.store(7, Ordering::Relaxed);

    register(
        Box::new(files) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default(),
    );

    let families = collected();
    let family = family_named(&families, "open_files");

    assert_eq!(
        family.metric[0]
            .gauge
            .as_ref()
            .and_then(|gauge| gauge.value),
        Some(7.0)
    );
}

/// Differentiating by label set through [`EncodeMetric`] alone means owning the
/// storage, the sharing, the label serialization, and the row assembly.
#[derive(Clone, Default)]
struct OpenFilesPerMount {
    values: Arc<RwLock<HashMap<Labels, Arc<AtomicU64>>>>,
}

impl OpenFilesPerMount {
    fn get_or_create(&self, labels: &Labels) -> Arc<AtomicU64> {
        if let Some(value) = self.values.read().unwrap().get(labels) {
            return Arc::clone(value);
        }

        Arc::clone(
            self.values
                .write()
                .unwrap()
                .entry(labels.clone())
                .or_default(),
        )
    }
}

impl EncodeMetric for OpenFilesPerMount {
    fn encode(&self) -> Vec<MetricFamily> {
        let values = self.values.read().unwrap();
        let mut rows = Vec::with_capacity(values.len());

        for (labels, value) in values.iter() {
            let Ok(label) = to_label_pairs(labels) else {
                continue;
            };

            rows.push(Metric {
                label,
                gauge: gauge(value.load(Ordering::Relaxed)),
                ..Default::default()
            });
        }

        vec![MetricFamily {
            name: Some("open_files_per_mount".to_owned()),
            help: Some("Open file descriptors.".to_owned()),
            r#type: Some(MetricType::Gauge as i32),
            metric: rows,
            unit: None,
        }]
    }
}

/// Implementing [`EncodeMetricValue`] instead delegates all of that to
/// [`Family`] and [`NamedMetric`]; only the value encoding remains.
#[derive(Default)]
struct OpenFilesValue(AtomicU64);

impl EncodeMetricValue for OpenFilesValue {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        vec![MetricFamily {
            // Relative name: `NamedMetric` prepends the registered name.
            name: Some(String::new()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![Metric {
                gauge: gauge(self.0.load(Ordering::Relaxed)),
                ..Default::default()
            }],
            unit: None,
        }]
    }
}

#[test]
fn both_label_set_approaches_produce_the_same_series() {
    let manual = OpenFilesPerMount::default();
    manual
        .get_or_create(&Labels { mount: "/data" })
        .store(3, Ordering::Relaxed);
    register(
        Box::new(manual.clone()) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default(),
    );

    let via_family = Family::<Labels, OpenFilesValue>::default();
    via_family
        .get_or_create(&Labels { mount: "/data" })
        .0
        .store(3, Ordering::Relaxed);
    register(
        Box::new(NamedMetric::new(
            "open_files_via_family",
            "Open file descriptors.",
            via_family,
        )) as Box<dyn EncodeMetric>,
        RegistrationMetadata::default(),
    );

    let families = collected();
    let manual = &family_named(&families, "open_files_per_mount").metric[0];
    let via_family = &family_named(&families, "open_files_via_family").metric[0];

    assert_eq!(manual.label, via_family.label);
    assert_eq!(
        manual.gauge.as_ref().and_then(|gauge| gauge.value),
        via_family.gauge.as_ref().and_then(|gauge| gauge.value)
    );
    assert_eq!(
        manual.gauge.as_ref().and_then(|gauge| gauge.value),
        Some(3.0)
    );
}
