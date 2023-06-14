//! Bedrock is a modular foundation for production-ready services implemented in Rust.
//!
//! If you need any of those:
//!
//! * logging
//! * distributed tracing
//! * metrics
//! * [seccomp] hardening
//! * service configuration with documentation
//! * graceful restarts primitives
//! * full application bootstraping that set up **any combination** of the above in a few lines of code
//!
//! then Bedrock is a tool of choice for you.
//!
//! # Features
//! Bedrock can take of all aspects of service bootstrapping, but also can be used as a component
//! library in a modular fashion by enabling or disabling [Cargo features]:
//!
//! - **default**: All features are enabled by default.
//! - **platform-common-default**: The same as **default**, but excludes platform-specific features,
//! such as **seccomp**.
//! - **settings**: Enables serializable documented settings functionality.
//! - **telemetry**: Enables all the telemetry-related features (**logging**, **tracing**).
//! - **logging**: Enables logging functionality.
//! - **tracing**: Enables distributed tracing functionality.
//! - **testing**: Enables testing-related functionality.
//! - **seccomp**: Enables [seccomp] hardening functionality. Available only on Linux (x86_64, aarch64).
//!
//! [Cargo features]: https://doc.rust-lang.org/stable/cargo/reference/features.html#the-features-section
//! [seccomp]: https://en.wikipedia.org/wiki/Seccomp

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod utils;

#[cfg(feature = "settings")]
pub mod settings;

#[cfg(any(feature = "logging", feature = "tracing"))]
pub mod telemetry;

#[cfg(all(
    feature = "seccomp",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub mod seccomp;

#[doc(hidden)]
pub mod reexports_for_macros {
    #[cfg(feature = "tracing")]
    pub use rustracing;

    #[cfg(feature = "settings")]
    pub use serde;

    #[cfg(feature = "logging")]
    pub use slog;

    #[cfg(feature = "seccomp")]
    pub use once_cell;
}

/// Error that can be returned on a service initialisation.
///
/// This is an alias for [`anyhow::Error`]. On bootstrap all errors are propagated to
/// the `main` function and eventually terminate the process. [Sentry] logs those errors on
/// application termination and in order to have proper understanding of the place where error
/// has occured we use `anyhow` errors that provide backtraces for error creation site.
///
/// [Sentry]: https://sentry.io
pub type BootstrapError = anyhow::Error;

/// Result that has [`BootstrapError`] as an error variant.
pub type BootstrapResult<T> = anyhow::Result<T>;

/// A generic operational (post-initialization) error without backtraces.
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Operational (post-initialization) result that has [`Error`] as an error variant.
pub type Result<T> = std::result::Result<T, Error>;

/// Basic service information.
#[derive(Copy, Clone, Debug, Default)]
pub struct ServiceInfo {
    /// The name of the service.
    pub name: &'static str,

    /// The version of the service.
    pub version: &'static str,

    /// The description of the service.
    pub description: &'static str,
}

/// Creates [`ServiceInfo`] from the information in `Cargo.toml` manifest of the service.
#[macro_export]
macro_rules! service_info {
    () => {
        $crate::ServiceInfo {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            description: env!("CARGO_PKG_DESCRIPTION"),
        }
    };
}
