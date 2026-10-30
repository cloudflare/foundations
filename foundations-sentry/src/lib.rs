#![allow(clippy::needless_doctest_main, reason = "for illustration")]
//! [`foundations`] companion crate with Sentry integrations. Includes rate limiting
//! of sentry events and tracking them with [`foundations`] metrics.
//!
//! This crate provides a sentry hook that increments the
//! `sentry_events_total{level=<...>}` metric for each sentry event. If a
//! previous `before_send` hook exists, it will be executed after the
//! per-fingerprint rate limit and before the global and fatal rate limits (see
//! below) and the metric. Only unfiltered events are counted.
//!
//! # Rate limiting
//!
//! All rate limits are disabled by default, and can be enabled in [`SentrySettings`]:
//!
//! - **Per fingerprint**: each group of events with the same fingerprint has a separate rate
//!   limit (`per_fingerprint_rate_limit`, or the `max_events_per_second` shorthand).
//! - **Global**: a single rate limit shared by all events (`global_rate_limit`).
//! - **Fatal**: a single rate limit shared by fatal events, which then don't count against the
//!   global one (`fatal_rate_limit`). This keeps a flood of other events from crowding out
//!   panics.
//!
//! An event is checked against the per-fingerprint rate limit first, so that the events of a
//! fingerprint that is already rate limited don't use up the global allowance. The global and
//! fatal rate limits are checked after the previous `before_send` hook, so that events it drops
//! don't use up their allowance either. Rate-limited events
//! are dropped, and counted by the `sentry_events_rate_limited_total{level=<...>,limit=<...>}`
//! metric.
//!
//! The fingerprint of a sentry event is the first out of the following attributes that is
//! present and not defaulted:
//!
//! 1. Explicit `event.fingerprint`
//! 2. Event message
//! 3. First exception value, or exception type if no value is set
//! 4. Fallback: event level name (e.g., `level::error`)
//!
//! Up to `max_tracked_fingerprints` fingerprints are tracked at once. Beyond that, events with
//! new fingerprints are only subject to the global and fatal rate limits, until idle fingerprints
//! are forgotten.
//!
//! The hook runs after Sentry's integrations have processed the event, so the rate limits don't
//! save the cost of e.g. capturing a stacktrace for dropped events.
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
//!
//! For instance, these settings allow 10 events per minute for each fingerprint, and 60 events
//! per minute in total, plus 60 fatal events per minute:
//!
//! ```rust
//! let sentry_settings: foundations_sentry::SentrySettings = foundations::settings::from_yaml_str(
//!     r#"
//!     per_fingerprint_rate_limit: { max_events: 10, period_secs: 60 }
//!     global_rate_limit: { max_events: 60, period_secs: 60 }
//!     fatal_rate_limit: { max_events: 60, period_secs: 60 }
//!     "#,
//! )
//! .unwrap();
//! # assert_eq!(sentry_settings.global_rate_limit.max_events, 60);
//! ```
//!
//! # Server-side symbolication
//!
//! Stacktraces are symbolicated locally by default. To capture instruction addresses
//! without loading debug information, replace Sentry's automatic stacktrace integration
//! with [`backtrace::UnresolvedStacktraceIntegration`] and enable
//! [`panic::NoFlushPanicIntegration::with_unresolved_stacktraces`].
//!
//! Also install `sentry_debug_images::DebugImagesIntegration` (available through Sentry's
//! `debug-images` feature) to attach loaded-image addresses and build IDs. The Sentry
//! server must have access to the matching debug information, for example via debuginfod.
//! These capture integrations alone do not provide the image metadata or debug files.
//!
//! Configure the capture integrations with:
//!
//! ```rust
//! use std::sync::Arc;
//! use foundations_sentry::backtrace::UnresolvedStacktraceIntegration;
//! use foundations_sentry::panic::NoFlushPanicIntegration;
//! use sentry_core::ClientOptions;
//!
//! let options = ClientOptions {
//!     attach_stacktrace: true,
//!     default_integrations: false,
//!     integrations: vec![
//!         Arc::new(UnresolvedStacktraceIntegration),
//!         Arc::new(NoFlushPanicIntegration::new().with_unresolved_stacktraces()),
//!     ],
//!     ..Default::default()
//! };
//! ```
//!
//! Other integrations and previously installed panic hooks can still resolve their own
//! stacktraces. Existing stacktraces on events are preserved, not replaced or resolved.

pub mod backtrace;
pub mod metrics;
pub mod panic;

mod hook;
mod rate_limit;
mod settings;

pub use self::hook::install_hook_with_settings;
pub use self::settings::{EventRateLimitSettings, SentrySettings};
