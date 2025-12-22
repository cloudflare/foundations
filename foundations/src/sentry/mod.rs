#![allow(clippy::needless_doctest_main)]
//! Sentry hook for tracking sentry events with metrics.
//!
//! This module provides a sentry hook that increments the
//! `sentry_events_total{level=<...>}` metric for each sentry event. If a
//! previous `before_send` hook exists, it will be executed before the installed
//! foundations hook. Only unfiltered events are counted.
//!
//! **note**: a clone of a client's [`sentry_core::ClientOptions`] will have the
//! hook installed. This means "child" sentry clients will still increment the
//! `sentry_events_total` metric. A reinstall is only required if the
//! [`sentry_core::ClientOptions::before_send`] field is overwritten.
//!
//! # Usage
//!
//! To install the hook:
//!
//! ```rust
//! fn main() {
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     foundations::sentry::install_hook(&mut client_opts);
//!     // sentry::init(client_opts);
//! }
//! ```

pub mod metrics;

mod hook;

pub use self::hook::install_hook;
pub use sentry_core::Level;
