//! foundations::telemetry::log::log_volume_metrics provides metrics collection based on log volume.

use crate::telemetry::metrics::Counter;
use slog::{Drain, OwnedKVList, Record};

#[crate::telemetry::metrics::metrics(crate_path = "crate")]
mod foundations {
    /// The number of produced log entries.
    pub fn log_record_count(level: &'static str) -> Counter;
}

/// LogVolumeMetricsDrain represents a Drain that updates log volume metrics for each log.
pub(crate) struct LogVolumeMetricsDrain<D> {
    inner: D,
}

impl<D: Drain> LogVolumeMetricsDrain<D> {
    /// Returns a new instance with wrapped Drain, ensuring that calling this LogVolumeMetricsDrain
    /// maintains functionality of wrapped Drain while incrementing log volume metrics.
    pub(crate) fn new(inner: D) -> Self {
        Self { inner }
    }
}

impl<D: Drain> Drain for LogVolumeMetricsDrain<D> {
    type Ok = D::Ok;
    type Err = D::Err;

    /// LogVolumeMetricsDrain simply increments the log volume metrics for each log.
    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let res = self.inner.log(record, values);
        foundations::log_record_count(record.level().as_str()).inc();
        res
    }

    #[inline]
    fn is_enabled(&self, level: slog::Level) -> bool {
        Drain::is_enabled(&self.inner, level)
    }

    #[inline]
    fn flush(&self) -> Result<(), slog::FlushError> {
        Drain::flush(&self.inner)
    }
}
