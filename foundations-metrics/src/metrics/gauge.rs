use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use foundations_metrics_registry::proto::{self, MetricType};

use crate::{MetricFamily, value::EncodeMetricValue};

use super::IntoF64;

/// A metric whose value may increase, decrease, or be set directly.
///
/// It is a cheap handle over shared atomic storage: [`Clone`] hands out another
/// reference to the *same* series, so the gauge can be updated from many places
/// and read back as a single current value.
///
/// # Examples
///
/// ```
/// use foundations_metrics::Gauge;
///
/// let connections: Gauge = Gauge::default();
/// connections.inc();
/// connections.inc_by(4);
/// connections.dec_by(2);
/// assert_eq!(connections.get(), 3);
///
/// // Clones share storage.
/// let alias = connections.clone();
/// alias.set(10);
/// assert_eq!(connections.get(), 10);
/// ```
#[derive(Debug)]
pub struct Gauge<N = u64, A = AtomicU64> {
    val: Arc<A>,
    marker: PhantomData<N>,
}

impl<N, A> Clone for Gauge<N, A> {
    fn clone(&self) -> Self {
        Self {
            val: Arc::clone(&self.val),
            marker: PhantomData,
        }
    }
}

impl<N, A: Default> Default for Gauge<N, A> {
    fn default() -> Self {
        Self {
            val: Arc::new(A::default()),
            marker: PhantomData,
        }
    }
}

/// Atomic storage backing a [`Gauge`].
///
/// Implemented for the numeric types a gauge can hold. Foundations provides
/// implementations over the standard library's 64-bit atomics (`i64`, `u64`,
/// and `f64`); downstream code may implement it for custom storage.
///
/// Every method returns the value held *before* the operation was applied.
pub trait GaugeAtomic<N> {
    /// Increases the value by one, returning the previous value.
    fn inc(&self) -> N;

    /// Increases the value by `v`, returning the previous value.
    fn inc_by(&self, v: N) -> N;

    /// Decreases the value by one, returning the previous value.
    fn dec(&self) -> N;

    /// Decreases the value by `v`, returning the previous value.
    fn dec_by(&self, v: N) -> N;

    /// Sets the value to `v`, returning the previous value.
    fn set(&self, v: N) -> N;

    /// Loads the current value.
    fn get(&self) -> N;
}

impl GaugeAtomic<i64> for AtomicI64 {
    #[inline]
    fn inc(&self) -> i64 {
        self.inc_by(1)
    }

    #[inline]
    fn inc_by(&self, v: i64) -> i64 {
        self.fetch_add(v, Ordering::Relaxed)
    }

    #[inline]
    fn dec(&self) -> i64 {
        self.dec_by(1)
    }

    #[inline]
    fn dec_by(&self, v: i64) -> i64 {
        self.fetch_sub(v, Ordering::Relaxed)
    }

    #[inline]
    fn set(&self, v: i64) -> i64 {
        self.swap(v, Ordering::Relaxed)
    }

    #[inline]
    fn get(&self) -> i64 {
        self.load(Ordering::Relaxed)
    }
}

impl GaugeAtomic<u64> for AtomicU64 {
    #[inline]
    fn inc(&self) -> u64 {
        self.inc_by(1)
    }

    #[inline]
    fn inc_by(&self, v: u64) -> u64 {
        self.fetch_add(v, Ordering::Relaxed)
    }

    #[inline]
    fn dec(&self) -> u64 {
        self.dec_by(1)
    }

    #[inline]
    fn dec_by(&self, v: u64) -> u64 {
        self.fetch_sub(v, Ordering::Relaxed)
    }

    #[inline]
    fn set(&self, v: u64) -> u64 {
        self.swap(v, Ordering::Relaxed)
    }

    #[inline]
    fn get(&self) -> u64 {
        self.load(Ordering::Relaxed)
    }
}

impl GaugeAtomic<f64> for AtomicU64 {
    #[inline]
    fn inc(&self) -> f64 {
        self.inc_by(1.0)
    }

    #[inline]
    fn inc_by(&self, v: f64) -> f64 {
        super::update_f64(self, |old| old + v)
    }

    #[inline]
    fn dec(&self) -> f64 {
        self.dec_by(1.0)
    }

    #[inline]
    fn dec_by(&self, v: f64) -> f64 {
        super::update_f64(self, |old| old - v)
    }

    #[inline]
    fn set(&self, v: f64) -> f64 {
        f64::from_bits(self.swap(v.to_bits(), Ordering::Relaxed))
    }

    #[inline]
    fn get(&self) -> f64 {
        f64::from_bits(self.load(Ordering::Relaxed))
    }
}

impl<N, A: GaugeAtomic<N>> Gauge<N, A> {
    /// Increases the gauge by one, returning the previous value.
    #[inline]
    pub fn inc(&self) -> N {
        self.val.inc()
    }

    /// Increases the gauge by `v`, returning the previous value.
    #[inline]
    pub fn inc_by(&self, v: N) -> N {
        self.val.inc_by(v)
    }

    /// Decreases the gauge by one, returning the previous value.
    #[inline]
    pub fn dec(&self) -> N {
        self.val.dec()
    }

    /// Decreases the gauge by `v`, returning the previous value.
    #[inline]
    pub fn dec_by(&self, v: N) -> N {
        self.val.dec_by(v)
    }

    /// Sets the gauge to `v`, returning the previous value.
    #[inline]
    pub fn set(&self, v: N) -> N {
        self.val.set(v)
    }

    /// Returns the current value.
    #[inline]
    pub fn get(&self) -> N {
        self.val.get()
    }

    /// Returns a reference to the underlying atomic storage.
    #[inline]
    pub fn inner(&self) -> &A {
        self.val.as_ref()
    }
}

impl<N, A> EncodeMetricValue for Gauge<N, A>
where
    N: IntoF64,
    A: GaugeAtomic<N> + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        vec![MetricFamily {
            name: Some(String::new()),
            help: None,
            r#type: Some(MetricType::Gauge as i32),
            metric: vec![proto::Metric {
                gauge: Some(proto::Gauge {
                    value: Some(self.get().into_f64()),
                }),
                ..Default::default()
            }],
            unit: None,
        }]
    }
}

/// A gauge that also records the minimum and maximum values seen since the last
/// scrape.
///
/// This gives visibility into the full range of a value within a scrape interval
/// with less overhead than a histogram. Reading the metric at encode time resets
/// the tracked minimum and maximum. It exports three series: the current value,
/// `_min`, and `_max`.
///
/// # Examples
///
/// ```ignore
/// use foundations_metrics::{metrics, RangeGauge};
///
/// #[metrics]
/// pub mod my_app_metrics {
///     /// Number of requests awaiting a response.
///     pub fn inflight_requests() -> RangeGauge;
/// }
///
/// fn usage() {
///     for _ in 0..10 {
///         my_app_metrics::inflight_requests().inc();
///     }
///     for _ in 0..8 {
///         my_app_metrics::inflight_requests().dec();
///     }
///
///     // If scraped now, the metric exports these three series:
///     // inflight_requests     2
///     // inflight_requests_min 0
///     // inflight_requests_max 10
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct RangeGauge {
    gauge: Gauge<u64, AtomicU64>,
    min: Arc<AtomicU64>,
    max: Arc<AtomicU64>,
    reset_cs: Arc<Mutex<()>>,
}

impl RangeGauge {
    /// Increases the gauge by one, returning the previous value.
    pub fn inc(&self) -> u64 {
        self.inc_by(1)
    }

    /// Increases the gauge by `v`, returning the previous value.
    pub fn inc_by(&self, v: u64) -> u64 {
        let prev = self.gauge.inc_by(v);
        self.update_max(prev + v);
        prev
    }

    /// Decreases the gauge by one, returning the previous value.
    pub fn dec(&self) -> u64 {
        self.dec_by(1)
    }

    /// Decreases the gauge by `v`, returning the previous value.
    pub fn dec_by(&self, v: u64) -> u64 {
        let prev = self.gauge.dec_by(v);
        self.update_min(prev - v);
        prev
    }

    /// Sets the gauge to `v`, returning the previous value.
    pub fn set(&self, v: u64) -> u64 {
        let prev = self.gauge.set(v);
        self.update_max(v);
        self.update_min(v);
        prev
    }

    /// Returns the current value of the gauge.
    pub fn get(&self) -> u64 {
        self.gauge.get()
    }

    /// Exposes the inner atomic backing the current value.
    ///
    /// This should only be used for advanced use-cases not directly supported by
    /// the library.
    pub fn inner(&self) -> &AtomicU64 {
        self.gauge.inner()
    }

    fn update_max(&self, new_max: u64) {
        self.max.fetch_max(new_max, Ordering::AcqRel);
    }

    fn update_min(&self, new_min: u64) {
        self.min.fetch_min(new_min, Ordering::AcqRel);
    }

    /// Returns `(min, current, max)`, guaranteeing `min <= current <= max`, and
    /// resets the tracked minimum and maximum to the current value.
    fn get_values(&self) -> (u64, u64, u64) {
        // Avoid data races by ensuring only one thread can perform the reset operation.
        let _reset_guard = self.reset_cs.lock().unwrap();

        // First, get the current metric.
        let current = self.get();

        // Obtain min and max by swapping their contents with the current value.
        // It is possible that current == min and another thread decremented current before we
        // read its value, but has not yet decremented min. Enforce min <= current to account for
        // that race. The same caveat applies to max.
        let min = std::cmp::min(current, self.min.swap(current, Ordering::AcqRel));
        let max = std::cmp::max(current, self.max.swap(current, Ordering::AcqRel));

        // The current value may have changed between reading it and resetting min/max. Read it
        // once more and ensure subsequent scrapes retain the invariant min <= current <= max.
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

/// Builds a single-row gauge `MetricFamily` with a relative name `suffix`.
fn gauge_family(suffix: &str, value: u64) -> MetricFamily {
    MetricFamily {
        name: Some(suffix.to_owned()),
        help: None,
        r#type: Some(MetricType::Gauge as i32),
        metric: vec![proto::Metric {
            gauge: Some(proto::Gauge {
                value: Some(value as f64),
            }),
            ..Default::default()
        }],
        unit: None,
    }
}

impl EncodeMetricValue for RangeGauge {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let (min, current, max) = self.get_values();

        vec![
            gauge_family("", current),
            gauge_family("_min", min),
            gauge_family("_max", max),
        ]
    }
}

/// Increments a gauge when created and decrements it when dropped.
///
/// Useful for tracking the number of in-progress operations: hold the guard for
/// the duration of the work and the gauge reflects the live count.
///
/// # Examples
///
/// ```ignore
/// use foundations_metrics::{metrics, Gauge, GaugeGuard};
///
/// #[metrics]
/// pub mod my_app_metrics {
///     /// Number of currently connected clients.
///     pub fn client_connections_active() -> Gauge;
/// }
///
/// fn usage() {
///     let client_metric = GaugeGuard::new(my_app_metrics::client_connections_active());
///     // Do work where you want the metric to remain incremented.
///     // When it leaves scope, the metric will be decremented.
///     // Alternatively, move ownership to another scope to change the lifetime.
///     tokio::spawn(async move {
///         // Do work with arbitrary lifetime on another task.
///         // Manually drop to force `client_metric` ownership to this task.
///         drop(client_metric);
///     });
/// }
/// ```
pub struct GaugeGuard<G: GenericGauge>(G);

impl<G: GenericGauge> GaugeGuard<G> {
    /// Creates a guard, incrementing the gauge now and decrementing it on drop.
    pub fn new(gauge: G) -> Self {
        gauge.inc();
        Self(gauge)
    }
}

impl<G: GenericGauge> Drop for GaugeGuard<G> {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// Helper trait for values supported by [`GaugeGuard`].
pub trait GenericGauge {
    /// Increases the wrapped gauge by one.
    fn inc(&self);

    /// Decreases the wrapped gauge by one.
    fn dec(&self);
}

impl GenericGauge for Gauge {
    fn inc(&self) {
        Gauge::inc(self);
    }

    fn dec(&self) {
        Gauge::dec(self);
    }
}

impl GenericGauge for RangeGauge {
    fn inc(&self) {
        RangeGauge::inc(self);
    }

    fn dec(&self) {
        RangeGauge::dec(self);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicU64};

    use super::*;

    fn encoded_value<N, A>(gauge: Gauge<N, A>) -> f64
    where
        N: IntoF64,
        A: GaugeAtomic<N> + Send + Sync + 'static,
    {
        let families = gauge.encode_metric_value();
        let family = &families[0];

        assert_eq!(family.r#type, Some(MetricType::Gauge as i32));
        assert_eq!(family.metric.len(), 1);

        family.metric[0]
            .gauge
            .as_ref()
            .and_then(|gauge| gauge.value)
            .expect("encoded gauge has a value")
    }

    #[test]
    fn default_gauge_uses_u64_and_clones_share_storage() {
        let gauge: Gauge = Gauge::default();
        let alias = gauge.clone();

        assert!(std::ptr::eq(gauge.inner(), alias.inner()));
        assert_eq!(gauge.set(4), 0);
        assert_eq!(alias.inc(), 4);
        assert_eq!(gauge.inc_by(3), 5);
        assert_eq!(alias.dec(), 8);
        assert_eq!(gauge.dec_by(2), 7);
        assert_eq!(alias.get(), 5);
    }

    #[test]
    fn encodes_64_bit_gauge_values() {
        let signed = Gauge::<i64, AtomicI64>::default();
        signed.set(-3);
        assert_eq!(encoded_value(signed), -3.0);

        let unsigned = Gauge::<u64, AtomicU64>::default();
        unsigned.set(7);
        assert_eq!(encoded_value(unsigned), 7.0);

        let float = Gauge::<f64, AtomicU64>::default();
        float.set(1.5);
        assert_eq!(encoded_value(float), 1.5);
    }

    /// Reads the encoded `(current, min, max)` triple; encoding resets min/max.
    #[track_caller]
    fn range_values(gauge: &RangeGauge) -> (u64, u64, u64) {
        let families = gauge.encode_metric_value();
        assert_eq!(families.len(), 3);
        assert_eq!(families[0].name.as_deref(), Some(""));
        assert_eq!(families[1].name.as_deref(), Some("_min"));
        assert_eq!(families[2].name.as_deref(), Some("_max"));

        let value = |family: &MetricFamily| {
            family.metric[0]
                .gauge
                .as_ref()
                .and_then(|gauge| gauge.value)
                .expect("encoded range gauge has a value") as u64
        };

        (
            value(&families[0]),
            value(&families[1]),
            value(&families[2]),
        )
    }

    #[test]
    fn range_gauge_tracks_and_resets_min_max() {
        let gauge = RangeGauge::default();

        assert_eq!(range_values(&gauge), (0, 0, 0));

        gauge.inc();
        assert_eq!(range_values(&gauge), (1, 0, 1));
        assert_eq!(range_values(&gauge), (1, 1, 1));

        gauge.dec();
        assert_eq!(range_values(&gauge), (0, 0, 1));
        assert_eq!(range_values(&gauge), (0, 0, 0));

        gauge.inc_by(3);
        gauge.dec_by(2);
        assert_eq!(range_values(&gauge), (1, 0, 3));

        gauge.inc_by(1);
        gauge.dec_by(2);
        assert_eq!(range_values(&gauge), (0, 0, 2));
    }

    #[test]
    fn gauge_guard_inc_on_new_dec_on_drop() {
        let gauge: Gauge = Gauge::default();
        {
            let _guard = GaugeGuard::new(gauge.clone());
            assert_eq!(gauge.get(), 1);
        }
        assert_eq!(gauge.get(), 0);
    }

    #[test]
    fn gauge_guard_supports_range_gauge() {
        let gauge = RangeGauge::default();
        {
            let _guard = GaugeGuard::new(gauge.clone());
            assert_eq!(gauge.get(), 1);
        }
        assert_eq!(gauge.get(), 0);
    }
}
