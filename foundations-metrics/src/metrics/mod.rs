//! The metrics module that contains scalar metrics (like Counter, Gauge, ...) and non-scalars,
//! like Histogram, NativeHistogram, and also Family.

use std::sync::atomic::{AtomicU64, Ordering};

mod counter;
mod gauge;

pub use counter::{Counter, CounterAtomic};
pub use gauge::{Gauge, GaugeAtomic, GaugeGuard, RangeGauge};

fn update_f64(atomic: &AtomicU64, f: impl Fn(f64) -> f64) -> f64 {
    let mut old_bits = atomic.load(Ordering::Relaxed);

    loop {
        let old = f64::from_bits(old_bits);
        let new_bits = f(old).to_bits();

        match atomic.compare_exchange_weak(old_bits, new_bits, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return old,
            Err(actual) => old_bits = actual,
        }
    }
}

trait IntoF64: Send + Sync + 'static {
    fn into_f64(self) -> f64;
}

macro_rules! impl_into_f64 {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl IntoF64 for $ty {
                fn into_f64(self) -> f64 {
                    self as f64
                }
            }
        )+
    };
}

impl_into_f64!(i64, u64, f64);
