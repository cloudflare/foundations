//! Gauge metrics and their lifecycle helpers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use foundations_metrics_registry::proto::{self, MetricFamily, MetricType};
use prometheus_client::metrics::gauge::{Atomic as GaugeAtomic, Gauge as PrometheusGauge};

use crate::value::EncodeMetricValue;

use super::IntoF64;

/// A metric whose value may increase, decrease, or be set directly.
///
/// This implementation delegates atomic storage and operations to
/// `prometheus-client`, while exposing only the explicit Foundations API.
/// Clones share the same underlying storage.
///
/// The `u64`/`AtomicU64` defaults preserve the existing Foundations API.
#[derive(Debug)]
#[repr(transparent)]
pub struct Gauge<N = u64, A = AtomicU64>(PrometheusGauge<N, A>);

impl<N, A> Clone for Gauge<N, A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<N, A: Default> Default for Gauge<N, A> {
    fn default() -> Self {
        Self(PrometheusGauge::default())
    }
}

impl<N, A: GaugeAtomic<N>> Gauge<N, A> {
    /// Increases the gauge by one, returning the previous value.
    #[inline]
    pub fn inc(&self) -> N {
        self.0.inc()
    }

    /// Increases the gauge by `v`, returning the previous value.
    #[inline]
    pub fn inc_by(&self, v: N) -> N {
        self.0.inc_by(v)
    }

    /// Decreases the gauge by one, returning the previous value.
    #[inline]
    pub fn dec(&self) -> N {
        self.0.dec()
    }

    /// Decreases the gauge by `v`, returning the previous value.
    #[inline]
    pub fn dec_by(&self, v: N) -> N {
        self.0.dec_by(v)
    }

    /// Sets the gauge to `v`, returning the previous value.
    #[inline]
    pub fn set(&self, v: N) -> N {
        self.0.set(v)
    }

    /// Returns the current value.
    #[inline]
    pub fn get(&self) -> N {
        self.0.get()
    }

    /// Returns a reference to the underlying atomic storage.
    ///
    /// This should only be used for advanced use-cases not directly supported by
    /// the library.
    #[inline]
    pub fn inner(&self) -> &A {
        self.0.inner()
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
/// ```
/// use foundations_metrics::RangeGauge;
///
/// let inflight = RangeGauge::default();
/// for _ in 0..10 {
///     inflight.inc();
/// }
/// for _ in 0..8 {
///     inflight.dec();
/// }
/// assert_eq!(inflight.get(), 2);
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
        let _reset_guard = self.reset_cs.lock().unwrap();
        let current = self.get();

        let min = std::cmp::min(current, self.min.swap(current, Ordering::AcqRel));
        let max = std::cmp::max(current, self.max.swap(current, Ordering::AcqRel));

        let current_fixup = self.get();
        self.min.fetch_min(current_fixup, Ordering::AcqRel);
        self.max.fetch_max(current_fixup, Ordering::AcqRel);

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
/// ```
/// use foundations_metrics::{Gauge, GaugeGuard};
///
/// let connections: Gauge = Gauge::default();
/// {
///     let _guard = GaugeGuard::new(connections.clone());
///     assert_eq!(connections.get(), 1);
/// }
/// assert_eq!(connections.get(), 0);
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
    /// Increments the gauge by one.
    fn inc(&self);

    /// Decrements the gauge by one.
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
    use std::sync::atomic::{AtomicI32, AtomicI64, AtomicU32, AtomicU64};

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
    fn encodes_32_bit_gauge_values() {
        let signed = Gauge::<i32, AtomicI32>::default();
        signed.set(-3);
        assert_eq!(encoded_value(signed), -3.0);

        let unsigned = Gauge::<u32, AtomicU32>::default();
        unsigned.set(7);
        assert_eq!(encoded_value(unsigned), 7.0);

        let float = Gauge::<f32, AtomicU32>::default();
        float.set(1.5);
        assert_eq!(encoded_value(float), 1.5);
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
