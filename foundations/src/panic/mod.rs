#![allow(clippy::needless_doctest_main)]
//! Panic hook for tracking panics with metrics.
//!
//! This module provides a panic hook that increments the `panics_total` metric
//! and logs the panic. If a previous panic hook exists, it will be executed
//! after the installed foundations hook.
//!
//! This does not require the `metrics` feature to be enabled. If foundations
//! users do not enable it, then a [`PanicsMetricsRegistry`] must be provided.
//!
//! # Usage
//!
//! Users of [`crate::telemetry::init()`] have the panic hook automatically
//! installed.
//!
//! To manually install the hook with the `metrics` feature enabled:
//!
//! ```rust
//! fn main() {
//!     foundations::panic::hook().init();
//! }
//! ```
//!
//! Without the `metrics` feature, you must provide a custom registry:
//!
//! ```rust,ignore
//! use foundations::panic::PanicsMetricsRegistry;
//!
//! struct MyRegistry;
//!
//! impl PanicsMetricsRegistry for MyRegistry {
//!     fn inc_panics_total(&self, by: u64) {
//!         // your implementation
//!     }
//! }
//!
//! fn main() {
//!     let registry = MyRegistry;
//!
//!     foundations::panic::hook()
//!         .with_registry(registry)
//!         .init();
//! }
//! ```

#[cfg(feature = "metrics")]
pub mod metrics;

mod hook;

#[cfg(feature = "metrics")]
use crate::registry_typestate::DefaultRegistry;
use crate::registry_typestate::{DefaultBuilderState, HasRegistry, NeedsRegistry};

pub use self::hook::{hook, PanicHookBuilder};

/// Trait for recording panic metrics.
///
/// Implement this trait to use a custom metrics registry instead of
/// `foundations::metrics`.
pub trait PanicsMetricsRegistry: Send + Sync {
    /// Increment the panics counter.
    fn inc_panics_total(&self, by: u64);
}

#[cfg(feature = "metrics")]
impl PanicsMetricsRegistry for DefaultRegistry {
    fn inc_panics_total(&self, by: u64) {
        metrics::panics::total().inc_by(by);
    }
}
