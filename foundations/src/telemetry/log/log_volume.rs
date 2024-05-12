//! foundations::telemetry::log::log_volume_metrics provides metrics collection based on log volume.

use crate::telemetry::metrics::Counter;
use slog::{Drain, Never, OwnedKVList, Record, SendSyncRefUnwindSafeDrain};

#[crate::telemetry::metrics::metrics(crate_path = "crate")]
mod foundations {
    /// The number of produced log entries.
    pub fn log_record_count(level: &'static str) -> Counter;
}

/// LogVolumeMetricsDrain represents a Drain that updates log volume metrics for each log.
pub(crate) struct LogVolumeMetricsDrain<D>
where
    D: SendSyncRefUnwindSafeDrain<Err = Never, Ok = ()> + 'static,
{
    inner: D,
}

impl<D> LogVolumeMetricsDrain<D>
where
    D: SendSyncRefUnwindSafeDrain<Err = Never, Ok = ()> + 'static,
{
    /// Returns a new instance with wrapped Drain, ensuring that calling this LogVolumeMetricsDrain
    /// maintains functionality of wrapped Drain while incrementing log volume metrics.
    pub(crate) fn new(inner: D) -> Self {
        Self { inner }
    }
}

impl<D> Drain for LogVolumeMetricsDrain<D>
where
    D: SendSyncRefUnwindSafeDrain<Err = Never, Ok = ()> + 'static,
{
    type Ok = ();
    type Err = D::Err;

    /// LogVolumeMetricsDrain simply increments the log volume metrics for each log.
    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let res = self.inner.log(record, values);
        foundations::log_record_count(record.level().as_str()).inc();
        res
    }
}
