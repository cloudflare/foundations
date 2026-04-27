#![allow(clippy::needless_doctest_main, reason = "for illustration")]
//! [`foundations`] companion crate with Sentry integrations. Includes rate limiting
//! of sentry events and tracking them with [`foundations`] metrics.
//!
//! This crate provides a sentry hook that increments the
//! `sentry_events_total{level=<...>}` metric for each sentry event. If a
//! previous `before_send` hook exists, it will be executed after rate limiting
//! and before the metric is incremented. Only unfiltered events are counted.
//!
//! For rate-limiting, we group events by fingerprint. Each group has a separate
//! rate limiter. The fingerprint of a sentry event is the first out of the following
//! attributes that is present and not defaulted:
//!
//! 1. Explicit `event.fingerprint`
//! 2. Event message
//! 3. First exception value, or exception type if no value is set
//! 4. Fallback: event level name (e.g., `error`)
//!
//! **note**: a clone of a client's [`sentry_core::ClientOptions`] will have the
//! hook installed. This means "child" sentry clients will inherit the hook. A
//! reinstall is only required if the [`sentry_core::ClientOptions::before_send`]
//! field is overwritten.
//!
//! # Usage
//!
//! To install the hook:
//!
//! ```rust
//! fn main() {
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     let sentry_settings = foundations_sentry::SentrySettings::default();
//!     foundations_sentry::install_hook_with_settings(&mut client_opts, &sentry_settings);
//!     // sentry::init(client_opts);
//! }
//! ```

pub mod metrics;

mod hook;
mod settings;

pub use self::hook::install_hook_with_settings;
pub use self::settings::SentrySettings;
