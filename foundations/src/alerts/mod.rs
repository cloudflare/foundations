#![allow(clippy::needless_doctest_main)]
//! Fatal error tracking for panics and sentry events.
//!
//! This module provides unified tracking of "fatal errors" which are events that
//! warrant human investigation.
//!
//! It includes:
//! - A panic hook that increments the `panics_total` metric and logs the panic
//! - A sentry hook that increments `sentry_events_total` metric (_requires the `sentry` feature_)
//!
//! If a previous panic or sentry hook exists, it will be executed after the
//! installed foundations hook.
//!
//! This does not require the `metrics` feature to be enabled. If foundations
//! users do not enable it, then a [`FatalErrorRegistry`] must be provided.
//!
//! # Usage
//!
//! Users of [`crate::telemetry::init()`] have the panic hook automatically
//! installed. However, the sentry hook still needs to be installed.
//!
//! To manually install the hooks with the `metrics` feature enabled:
//!
//! ```rust
//! fn main() {
//!     foundations::alerts::panic_hook().init();
//!
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     foundations::alerts::sentry_hook().install(&mut client_opts);
//!     // sentry::init(client_opts);
//! }
//! ```
//!
//! Without the `metrics` feature, you must provide a custom registry:
//!
//! ```rust,ignore
//! struct MyRegistry;
//!
//! fn main() {
//!     let registry = MyRegistry;
//!
//!     foundations::alerts::panic_hook()
//!         .with_registry(registry)
//!         .init();
//!
//!     let mut client_opts = sentry_core::ClientOptions::default();
//!     foundations::alerts::sentry_hook()
//!         .with_registry(registry)
//!         .install(&mut client_opts);
//!     // sentry::init(client_opts);
//! }
//! ```

#[cfg(feature = "metrics")]
pub mod metrics;
mod panic;
#[cfg(feature = "sentry")]
mod sentry;

pub use self::panic::{panic_hook, PanicHookBuilder};

#[cfg(feature = "sentry")]
pub use self::sentry::{sentry_hook, SentryHookBuilder};

/// Sentry event severity level for metrics labeling.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(missing_docs)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

#[cfg(feature = "sentry")]
impl From<sentry_core::Level> for Level {
    fn from(level: sentry_core::Level) -> Self {
        match level {
            sentry_core::Level::Debug => Level::Debug,
            sentry_core::Level::Info => Level::Info,
            sentry_core::Level::Warning => Level::Warning,
            sentry_core::Level::Error => Level::Error,
            sentry_core::Level::Fatal => Level::Fatal,
        }
    }
}

#[cfg(feature = "sentry")]
impl From<Level> for sentry_core::Level {
    fn from(level: Level) -> Self {
        match level {
            Level::Debug => sentry_core::Level::Debug,
            Level::Info => sentry_core::Level::Info,
            Level::Warning => sentry_core::Level::Warning,
            Level::Error => sentry_core::Level::Error,
            Level::Fatal => sentry_core::Level::Fatal,
        }
    }
}

/// Trait for recording sentry and panic hook metrics.
///
/// Implement this trait to use a custom metrics registry instead of
/// `foundations::metrics`.
pub trait FatalErrorRegistry: Send + Sync {
    /// Increment the panics counter.
    fn inc_panics_total(&self, by: u64);

    /// Increment the sentry events counter.
    fn inc_sentry_events_total(&self, level: Level, by: u64);
}

#[doc(hidden)]
pub mod _private {
    /// The default registry implementation using foundations metrics.
    #[cfg(feature = "metrics")]
    pub struct DefaultRegistry {
        pub(crate) _private: (),
    }

    #[cfg(feature = "metrics")]
    impl super::FatalErrorRegistry for DefaultRegistry {
        fn inc_panics_total(&self, by: u64) {
            super::metrics::panics::total().inc_by(by);
        }

        fn inc_sentry_events_total(&self, level: super::Level, by: u64) {
            super::metrics::sentry_events::total(level).inc_by(by);
        }
    }

    #[derive(Default)]
    pub struct NeedsRegistry {
        pub(crate) _private: (),
    }

    pub struct HasRegistry<R> {
        pub(crate) registry: R,
    }

    #[cfg(feature = "metrics")]
    impl Default for HasRegistry<DefaultRegistry> {
        fn default() -> Self {
            Self {
                registry: DefaultRegistry { _private: () },
            }
        }
    }

    #[cfg(feature = "metrics")]
    pub type DefaultBuilderState = HasRegistry<DefaultRegistry>;

    #[cfg(not(feature = "metrics"))]
    pub type DefaultBuilderState = NeedsRegistry;
}
