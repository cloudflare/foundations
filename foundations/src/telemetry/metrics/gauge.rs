use prometheus_client::encoding::text::{EncodeMetric, Encoder};
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::{MetricType, TypedMetric};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Prometheus metric based on a gauge, but additionally records the minimum and maximum values of
/// that gauge since the last recorded value was taken.
///
/// This allows a user of the metric to see the full range of values within a smaller timespan with
/// greater precision and less overhead than a histogram. If the details of the intermediate values
/// are required, the histogram remains a more appropriate choice.
///
/// # Example
///
/// ```
/// # // See main example in mod.rs for why we do this.
/// # mod rustdoc_workaround {
/// use foundations::telemetry::metrics::{metrics, RangeGauge};
///
/// #[metrics]
/// pub mod my_app_metrics {
///     /// Number of requests awaiting a response
///     pub fn inflight_requests() -> RangeGauge;
/// }
///
/// fn usage() {
///     for _ in 0..10 {
///         my_app_metrics::inflight_requests().inc();
///     }
///
///     for _ in 0..8 {
///         my_app_metrics::inflight_requests().dec();
///     }
///
///     // If scraped at this point, the metric will export the following series:
///     // inflight_requests     2
///     // inflight_requests_min 0
///     // inflight_requests_max 10
/// }
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct RangeGauge {
    gauge: Gauge<u64, AtomicU64>,
    min: Arc<AtomicU64>,
    max: Arc<AtomicU64>,
    reset_cs: Arc<Mutex<()>>,
}

impl RangeGauge {
    /// Increase the [`RangeGauge`] by 1, returning the previous value.
    pub fn inc(&self) -> u64 {
        self.inc_by(1)
    }

    /// Increase the [`RangeGauge`] by `v`, returning the previous value.
    pub fn inc_by(&self, v: u64) -> u64 {
        let prev = self.gauge.inc_by(v);
        self.update_max(prev + v);
        prev
    }

    /// Decrease the [`RangeGauge`] by 1, returning the previous value.
    pub fn dec(&self) -> u64 {
        self.dec_by(1)
    }

    /// Decrease the [`RangeGauge`] by `v`, returning the previous value.
    pub fn dec_by(&self, v: u64) -> u64 {
        let prev = self.gauge.dec_by(v);
        self.update_min(prev - v);
        prev
    }

    /// Sets the [`RangeGauge`] to `v`, returning the previous value.
    pub fn set(&self, v: u64) -> u64 {
        let prev = self.gauge.set(v);
        self.update_max(v);
        self.update_min(v);
        prev
    }

    /// Get the current value of the [`RangeGauge`].
    pub fn get(&self) -> u64 {
        self.gauge.get()
    }

    /// Exposes the inner atomic type of the [`RangeGauge`].
    ///
    /// This should only be used for advanced use-cases which are not directly
    /// supported by the library.
    pub fn inner(&self) -> &AtomicU64 {
        self.gauge.inner()
    }

    fn update_max(&self, new_max: u64) {
        self.max.fetch_max(new_max, Ordering::AcqRel);
    }

    fn update_min(&self, new_min: u64) {
        self.min.fetch_min(new_min, Ordering::AcqRel);
    }

    /// Get the minimum, current and maximum values in that order.
    /// The return value ensures min <= current <= max.
    /// The minimum and maximum values are reset.
    fn get_values(&self) -> (u64, u64, u64) {
        // Avoid data races by ensuring only one thread can perform the 'reset' operation.
        // The previous current value is stored.
        let _reset_guard = self.reset_cs.lock().unwrap();
        // First step is to get the current metric.
        let current = self.get();
        // Second step is to obtain min and max by swapping their contents with the "current" value.
        // DATA RACE: It is possible that current == min, and another thread decremented current
        // before we read its value, but has not yet decremented min. So, enforce the invariant that
        // min <= current.
        let min = std::cmp::min(current, self.min.swap(current, Ordering::AcqRel));
        // DATA RACE: Same caveat as above applies to max.
        let max = std::cmp::max(current, self.max.swap(current, Ordering::AcqRel));
        // It is possible that the current value was incremented or decremented between us getting
        // the value in step 1 and setting the min/max values in step 2.
        // In this case, the current value will exceed the bounds suggested by min/max.
        // Let's fix this up by getting the current value once more and enforcing the invariant that
        // min <= current <= max.
        let current_fixup = self.get();
        self.min.fetch_min(current_fixup, Ordering::AcqRel);
        self.max.fetch_max(current_fixup, Ordering::AcqRel);
        //                     | min | c | max |
        // T1: read current    | 1   | 1 | 1   |
        // T2: increment by 1  | 1   | 2 | 2   |
        // T3: decrement by 2  | 0   | 0 | 2   |

        (min, current, max)
    }
}

impl TypedMetric for RangeGauge {
    const TYPE: MetricType = MetricType::Gauge;
}

impl EncodeMetric for RangeGauge {
    fn encode(&self, mut encoder: Encoder) -> Result<(), std::io::Error> {
        let (min, current, max) = self.get_values();

        encoder
            .no_suffix()?
            .no_bucket()?
            .encode_value(current)?
            .no_exemplar()?;

        encoder
            .encode_suffix("min")?
            .no_bucket()?
            .encode_value(min)?
            .no_exemplar()?;

        encoder
            .encode_suffix("max")?
            .no_bucket()?
            .encode_value(max)?
            .no_exemplar()?;

        Ok(())
    }

    fn metric_type(&self) -> MetricType {
        Self::TYPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::encoding::text::encode;
    use prometheus_client::registry::Registry;

    struct MetricValueHelper(Registry<RangeGauge>);

    impl MetricValueHelper {
        fn new(metric: &RangeGauge) -> Self {
            let mut reg = Registry::default();
            reg.register("mygauge", "", metric.clone());
            Self(reg)
        }

        #[track_caller]
        fn assert_values(&self, val: u64, min: u64, max: u64) {
            let mut encoded = vec![];
            encode(&mut encoded, &self.0).unwrap();
            assert_eq!(
                std::str::from_utf8(&encoded).unwrap(),
                format!(
                    "\
# HELP mygauge .
# TYPE mygauge gauge
mygauge {val}
mygauge_min {min}
mygauge_max {max}
# EOF
"
                ),
            );
        }
    }

    #[test]
    fn test_rangegauge_values() {
        let rg = RangeGauge::default();
        let helper = MetricValueHelper::new(&rg);

        helper.assert_values(0, 0, 0);
        rg.inc();
        helper.assert_values(1, 0, 1);
        // the act of observing the value should reset the min/max history
        helper.assert_values(1, 1, 1);
        rg.dec();
        helper.assert_values(0, 0, 1);
        // the act of observing the value should reset the min/max history
        helper.assert_values(0, 0, 0);
        // check that max continues to observe the highest seen value after the value goes down
        rg.inc_by(3);
        rg.dec_by(2);
        helper.assert_values(1, 0, 3);
        // change both min and max in one sample period
        rg.inc_by(1);
        rg.dec_by(2);
        helper.assert_values(0, 0, 2);
    }
}
