#![allow(clippy::needless_doctest_main)]
//! Panic hook for tracking panics with metrics.
//!
//! This module provides a panic hook that increments the `panics_total` metric
//! and logs the panic. If a previous panic hook exists, it will be executed
//! after the installed foundations hook.
//!
//! # Usage
//!
//! Users of [`crate::telemetry::init()`] have the panic hook automatically
//! installed.
//!
//! To manually install the hook:
//!
//! ```rust
//! fn main() {
//!     foundations::panic::install_hook();
//! }
//! ```

pub mod metrics;

mod hook;

pub use self::hook::install_hook;
