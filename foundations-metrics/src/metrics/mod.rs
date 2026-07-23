//! The metrics module that contains scalar metrics (like Counter, Gauge, ...) and non-scalars,
//! like Histogram, NativeHistogram, and also Family.
//!
//! # License
//!
//! Substantial parts of the code in this module were adapted from
//! [`prometheus-client`] subject to their licensing terms:
//!
//! ```text
//! MIT License
//!
//! Copyright (c) 2020 Max Inden
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.
//! ```
//!
//! [`prometheus-client`]: https://github.com/prometheus/client_rust

use std::sync::atomic::{AtomicU64, Ordering};

mod counter;
mod exemplar;
mod family;
mod gauge;
mod histogram;
mod native_histogram;

pub use counter::{Counter, CounterAtomic};
pub use exemplar::{
    CounterWithExemplar, Exemplar, HistogramWithExemplars, NativeHistogramWithExemplars,
    NativeHistogramWithExemplarsBuilder,
};
pub use family::{Family, FamilyMetricGuard, MetricConstructor};
pub use gauge::{Gauge, GaugeAtomic, GaugeGuard, RangeGauge};
pub use histogram::{
    Histogram, HistogramBuilder, HistogramSnapshot, HistogramTimer, TimeHistogram,
};
pub use native_histogram::{NativeHistogram, NativeHistogramBuilder};

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
