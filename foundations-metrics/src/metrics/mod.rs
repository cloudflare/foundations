//! The metrics module that contains scalar metrics (like Counter, Gauge, ...) and non-scalars, like Family.

mod counter;

pub use counter::{Atomic, Counter};

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

impl_into_f64!(i32, u32, f32, i64, u64, f64);
