use parking_lot::RwLock;
use std::sync::{Arc, OnceLock};

use crate::EncodeMetric;
use crate::RegistrationMetadata;
use crate::iter::MetricsIter;

/// A registered metric paired with its registration metadata
pub(crate) struct Entry {
    pub(crate) metadata: RegistrationMetadata,
    pub(crate) metric: Box<dyn EncodeMetric>,
}

static REGISTRY: OnceLock<RwLock<Vec<Arc<Entry>>>> = OnceLock::new();

fn registry() -> &'static RwLock<Vec<Arc<Entry>>> {
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Registers one or more metrics with the given metadata.
///
/// Registration is append-only; there is no unregister. Service-name handling
/// deliberately happens at encode time, not here, so metrics can be registered
/// before the service name is known.
pub fn register(metrics: impl IntoMetrics, metadata: RegistrationMetadata) {
    let mut guard = registry().write();
    for metric in metrics.into_metrics() {
        let entry: Arc<Entry> = Arc::new(Entry {
            metadata: metadata.clone(),
            metric,
        });
        guard.push(entry);
    }
}

/// Returns a snapshot iterator over the registered metrics and their metadata.
pub fn iter() -> MetricsIter {
    let entries: Vec<Arc<Entry>> = registry().read().clone();
    MetricsIter::new(entries)
}

mod private {
    pub trait Sealed {}
}

/// Converts a value into the metrics it contributes to the registry.
pub trait IntoMetrics: private::Sealed {
    /// Consumes `self`, yielding the metrics to be registered.
    fn into_metrics(self) -> Vec<Box<dyn EncodeMetric>>;
}

impl private::Sealed for Box<dyn EncodeMetric> {}
impl IntoMetrics for Box<dyn EncodeMetric> {
    fn into_metrics(self) -> Vec<Box<dyn EncodeMetric>> {
        vec![self]
    }
}

impl private::Sealed for Vec<Box<dyn EncodeMetric>> {}
impl IntoMetrics for Vec<Box<dyn EncodeMetric>> {
    fn into_metrics(self) -> Vec<Box<dyn EncodeMetric>> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Metric, MetricFamily, MetricType};

    struct Named(&'static str);

    impl EncodeMetric for Named {
        fn encode(&self) -> Vec<MetricFamily> {
            vec![MetricFamily {
                name: Some(self.0.to_owned()),
                help: Some("help.".to_owned()),
                r#type: Some(MetricType::Counter as i32),
                metric: vec![Metric::default()],
                unit: Some("seconds".to_owned()),
            }]
        }
    }

    #[test]
    fn register_and_iter_with_metadata() {
        register(
            Box::new(Named("required_metric")) as Box<dyn EncodeMetric>,
            RegistrationMetadata::default(),
        );
        register(
            vec![Box::new(Named("optional_metric")) as Box<dyn EncodeMetric>],
            RegistrationMetadata::default().optional(true),
        );

        let observed: Vec<(bool, Option<String>)> = iter()
            .map(|reg| {
                (
                    reg.metadata().optional,
                    reg.metric().encode()[0].name.clone(),
                )
            })
            .collect();

        assert!(observed.contains(&(false, Some("required_metric".to_owned()))));
        assert!(observed.contains(&(true, Some("optional_metric".to_owned()))));
    }
}
