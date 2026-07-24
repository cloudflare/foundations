use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use foundations_metrics_registry::proto::{self, MetricType};

use super::IntoF64;

use crate::{MetricFamily, value::EncodeMetricValue};

/// A monotonically increasing value, such as a request count or bytes served.
///
/// A [`Counter`] may only ever be incremented; it never decreases (aside from a
/// reset to zero on process restart). It is a cheap handle over shared atomic
/// storage: [`Clone`] hands out another reference to the *same* series, so the
/// same counter can be incremented from many places and read back as a single
/// total.
///
/// # Examples
///
/// ```
/// use foundations_metrics::Counter;
///
/// let requests: Counter = Counter::default();
/// requests.inc();
/// requests.inc_by(4);
/// assert_eq!(requests.get(), 5);
///
/// // Clones share storage.
/// let alias = requests.clone();
/// alias.inc();
/// assert_eq!(requests.get(), 6);
/// ```
#[derive(Debug)]
pub struct Counter<N = u64, A = AtomicU64> {
    val: Arc<A>,
    marker: PhantomData<N>,
}

/// Atomic storage backing a [`Counter`].
///
/// Implemented for the numeric types a counter can hold. Foundations provides
/// implementations for `u64` and `f64` over [`AtomicU64`]; downstream code may
/// implement it for custom storage.
pub trait CounterAtomic<N> {
    /// Increments the value by one, returning the previous value.
    fn inc(&self) -> N;

    /// Increments the value by `v`, returning the previous value.
    fn inc_by(&self, v: N) -> N;

    /// Loads the current value.
    fn get(&self) -> N;
}

impl CounterAtomic<u64> for AtomicU64 {
    #[inline]
    fn inc(&self) -> u64 {
        self.inc_by(1)
    }

    #[inline]
    fn inc_by(&self, v: u64) -> u64 {
        self.fetch_add(v, Ordering::Relaxed)
    }

    #[inline]
    fn get(&self) -> u64 {
        self.load(Ordering::Relaxed)
    }
}

impl CounterAtomic<f64> for AtomicU64 {
    #[inline]
    fn inc(&self) -> f64 {
        self.inc_by(1.0)
    }

    #[inline]
    fn inc_by(&self, v: f64) -> f64 {
        super::update_f64(self, |old| old + v)
    }

    #[inline]
    fn get(&self) -> f64 {
        f64::from_bits(self.load(Ordering::Relaxed))
    }
}

impl<N, A: CounterAtomic<N>> Counter<N, A> {
    /// Increments the counter by one, returning the previous value.
    #[inline]
    pub fn inc(&self) -> N {
        self.val.inc()
    }

    /// Increments the counter by `v`, returning the previous value.
    #[inline]
    pub fn inc_by(&self, v: N) -> N {
        self.val.inc_by(v)
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

impl<N, A> Clone for Counter<N, A> {
    fn clone(&self) -> Self {
        Self {
            val: Arc::clone(&self.val),
            marker: PhantomData,
        }
    }
}

impl<N, A: Default> Default for Counter<N, A> {
    fn default() -> Self {
        Self {
            val: Arc::new(A::default()),
            marker: PhantomData,
        }
    }
}

/// Builds the protobuf `MetricFamily` for a counter value.
///
/// The name/help are left empty here; they are populated at registration and
/// encode time.
pub(super) fn encode_counter(value: f64) -> Vec<MetricFamily> {
    vec![MetricFamily {
        name: Some(String::new()),
        help: None,
        r#type: Some(MetricType::Counter as i32),
        metric: vec![proto::Metric {
            counter: Some(proto::Counter {
                value: Some(value),
                ..Default::default()
            }),
            ..Default::default()
        }],
        unit: None,
    }]
}

impl<N, A> EncodeMetricValue for Counter<N, A>
where
    N: IntoF64,
    A: CounterAtomic<N> + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        encode_counter(self.get().into_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_value<N, A>(counter: Counter<N, A>) -> f64
    where
        N: IntoF64,
        A: CounterAtomic<N> + Send + Sync + 'static,
    {
        let families = counter.encode_metric_value();
        let family = &families[0];

        assert_eq!(family.r#type, Some(MetricType::Counter as i32));
        assert_eq!(family.metric.len(), 1);

        family.metric[0]
            .counter
            .as_ref()
            .and_then(|counter| counter.value)
            .expect("encoded counter has a value")
    }

    #[test]
    fn inc_inc_by_and_get() {
        let counter = Counter::<u64>::default();
        assert_eq!(counter.get(), 0);

        assert_eq!(counter.inc(), 0);
        assert_eq!(counter.get(), 1);

        assert_eq!(counter.inc_by(4), 1);
        assert_eq!(counter.get(), 5);
    }

    #[test]
    fn f64_counter_inc_and_get() {
        let counter = Counter::<f64>::default();
        assert_eq!(counter.get(), 0.0);

        assert_eq!(counter.inc(), 0.0);
        assert_eq!(counter.inc_by(1.5), 1.0);
        assert_eq!(counter.get(), 2.5);
    }

    #[test]
    fn clones_share_storage() {
        let counter: Counter = Counter::default();
        let alias = counter.clone();

        counter.inc();
        alias.inc();

        assert_eq!(counter.get(), 2);
        assert_eq!(alias.get(), 2);
    }

    #[test]
    fn encodes_counter_values() {
        let unsigned = Counter::<u64>::default();
        unsigned.inc_by(7);
        assert_eq!(encoded_value(unsigned), 7.0);

        let float = Counter::<f64>::default();
        float.inc_by(1.5);
        assert_eq!(encoded_value(float), 1.5);
    }
}
