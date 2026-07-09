use std::sync::Arc;

use crate::EncodeMetric;
use crate::RegistrationMetadata;
use crate::registry::Entry;

/// A registered metric together with its metadata.
///
/// Yielded by [`MetricsIter`]
pub struct MetricRegistration {
    entry: Arc<Entry>,
}

impl MetricRegistration {
    /// The metadata supplied to [`register`](crate::register).
    pub fn metadata(&self) -> &RegistrationMetadata {
        &self.entry.metadata
    }
    /// The registered metric.
    pub fn metric(&self) -> &dyn EncodeMetric {
        self.entry.metric.as_ref()
    }
}

/// A point-in-time snapshot iterator over the registered metrics.
///
/// Metrics registered after [`iter`](crate::iter()) was called are not observed,
/// and the registry lock is not held while iterating.
pub struct MetricsIter {
    entries: std::vec::IntoIter<Arc<Entry>>,
}

impl MetricsIter {
    pub(crate) fn new(entries: Vec<Arc<Entry>>) -> Self {
        Self {
            entries: entries.into_iter(),
        }
    }
}

impl Iterator for MetricsIter {
    type Item = MetricRegistration;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;

        Some(MetricRegistration { entry })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for MetricsIter {}
