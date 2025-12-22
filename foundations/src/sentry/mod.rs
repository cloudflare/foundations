#![allow(clippy::needless_doctest_main)]
//! Sentry hook for tracking sentry events with metrics.
//!
//! This module provides a sentry hook that increments the
//! `sentry_events_total{level=<...>}` metric for each sentry event. If a
//! previous `before_send` hook exists, it will be executed before the installed
//! foundations hook. Only unfiltered events are counted.
//!
//! This does not require the `metrics` feature to be enabled. If foundations
//! users do not enable it, then a [`SentryMetricsRegistry`] must be provided.
//!
//! **note**: a clone of a client's [`sentry_core::ClientOptions`] will have the
//! hook installed. This means "child" sentry clients will still increment the
//! `sentry_events_total` metric. A reinstall is only required if the
//! [`sentry_core::ClientOptions::before_send`] field is overwritten.
//!
//! # Usage
//!
//! To install the hook with the `metrics` feature enabled:
//!
//! ```rust
//! fn main() {
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     foundations::sentry::hook().install(&mut client_opts);
//!     // sentry::init(client_opts);
//! }
//! ```
//!
//! Without the `metrics` feature, you must provide a custom registry:
//!
//! ```rust,ignore
//! use foundations::sentry::{SentryMetricsRegistry, Level};
//!
//! struct MyRegistry;
//!
//! impl SentryMetricsRegistry for MyRegistry {
//!     fn inc_sentry_events_total(&self, level: Level, by: u64) {
//!         // your implementation
//!     }
//! }
//!
//! fn main() {
//!     let registry = MyRegistry;
//!
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     foundations::sentry::hook()
//!         .with_registry(registry)
//!         .install(&mut client_opts);
//!     // sentry::init(client_opts);
//! }
//! ```

#[cfg(feature = "metrics")]
pub mod metrics;

mod hook;

#[cfg(feature = "metrics")]
use crate::registry_typestate::DefaultRegistry;
use crate::registry_typestate::{DefaultBuilderState, HasRegistry, NeedsRegistry};

pub use self::hook::{hook, SentryHookBuilder};
pub use sentry_core::Level;

/// Trait for recording sentry event metrics.
///
/// Implement this trait to use a custom metrics registry instead of
/// `foundations::metrics`.
pub trait SentryMetricsRegistry: Send + Sync {
    /// Increment the sentry events counter.
    fn inc_sentry_events_total(&self, level: Level, by: u64);
}

#[cfg(feature = "metrics")]
impl SentryMetricsRegistry for DefaultRegistry {
    fn inc_sentry_events_total(&self, level: Level, by: u64) {
        metrics::sentry_events::total(level).inc_by(by);
    }
}
