//! Bedrock is a modular foundation for production-ready services implemented in Rust.
//!
//! If you need any of those:
//!
//! * logging
//! * distributed tracing
//! * metrics
//! * memory profiling and [jemalloc] allocator
//! * security features, such as [seccomp]-based syscall sandboxing
//! * service configuration with documentation
//! * CLI helper that takes care of the configuration loading
//!
//! then Bedrock is a tool of choice for you.
//!
//! Check out `/examples` for an example of how all these components can be used together.
//!
//! # Features
//! Bedrock can take of all aspects of service bootstrapping, but also can be used as a component
//! library in a modular fashion by enabling or disabling [Cargo features]:
//!
//! - **default**: All features are enabled by default.
//! - **platform-common-default**: The same as **default**, but excludes platform-specific features,
//! such as **security**.
//! - **settings**: Enables serializable documented settings functionality.
//! - **telemetry**: Enables all the telemetry-related features (**metrics**, **logging**, **tracing**, **telemetry-server**).
//! - **telemetry-server**: Enables the telemetry server.
//! - **metrics**: Enables metrics functionality.
//! - **logging**: Enables logging functionality.
//! - **tracing**: Enables distributed tracing functionality.
//! - **testing**: Enables testing-related functionality.
//! - **security**: Enables security features. Available only on Linux (x86_64, aarch64).
//! - **jemalloc**: Enables [jemalloc] memory allocator which is known to perform much better than
//! system allocators for long living service.
//! - **memory-profiling**: Enables memory profiling functionality and telemetry. Implicity enables
//!  **jemalloc** feature.
//! - **cli**: Enables command line interface (CLI) functionality. Implicitly enabled **settings**
//! feature.
//!
//! [Cargo features]: https://doc.rust-lang.org/stable/cargo/reference/features.html#the-features-section
//! [seccomp]: https://en.wikipedia.org/wiki/Seccomp
//! [jemalloc]: https://github.com/jemalloc/jemalloc

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod utils;

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "settings")]
pub mod settings;

#[cfg(any(
    feature = "logging",
    feature = "metrics",
    feature = "telemetry",
    feature = "tracing"
))]
pub mod telemetry;

#[cfg(all(
    feature = "security",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub mod security;

#[doc(hidden)]
pub mod reexports_for_macros {
    #[cfg(any(feature = "metrics", feature = "security"))]
    pub use once_cell;
    #[cfg(feature = "metrics")]
    pub use parking_lot;
    #[cfg(feature = "metrics")]
    pub use prometheus_client;
    #[cfg(feature = "metrics")]
    pub use prometools;
    #[cfg(feature = "tracing")]
    pub use rustracing;
    #[cfg(any(feature = "metrics", feature = "settings"))]
    pub use serde;
    #[cfg(feature = "metrics")]
    pub use serde_with;
    #[cfg(feature = "logging")]
    pub use slog;
}

/// Global memory allocator backed by [jemalloc].
///
/// This static variable is exposed solely for the documentation purposes and don't need to be used
/// directly. If **jemalloc** feature is enabled then the service will use jemalloc for all the
/// memory allocations implicitly.
///
/// [jemalloc]: https://github.com/jemalloc/jemalloc
#[cfg(feature = "jemalloc")]
#[global_allocator]
pub static JEMALLOC_MEMORY_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Error that can be returned on a service initialisation.
///
/// This is an alias for [`anyhow::Error`]. On service bootstrap all such errors can be
/// propagated to the `main` function and eventually terminate the process. [Sentry] logs those
/// errors on application termination and in order to have proper understanding of the place where
/// error has occured we use `anyhow` errors that provide backtraces for error creation site.
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
#[derive(Clone, Debug, Default)]
pub struct ServiceInfo {
    /// The name of the service.
    pub name: &'static str,

    /// The service identifier as used in metrics.
    ///
    /// Usually the same as [`ServiceInfo::name`], with hyphens (`-`) replaced by underscores `_`.
    pub name_in_metrics: String,

    /// The version of the service.
    pub version: &'static str,

    /// Service author.
    pub author: &'static str,

    /// The description of the service.
    pub description: &'static str,
}

/// Creates [`ServiceInfo`] from the information in `Cargo.toml` manifest of the service.
///
/// [`ServiceInfo::name_in_metrics`] is the same as the package name, with hypens (`-`) replaced
/// by underscores (`_`).
#[macro_export]
macro_rules! service_info {
    () => {
        $crate::ServiceInfo {
            name: env!("CARGO_PKG_NAME"),
            name_in_metrics: env!("CARGO_PKG_NAME").replace("-", "_"),
            version: env!("CARGO_PKG_VERSION"),
            author: env!("CARGO_PKG_AUTHORS"),
            description: env!("CARGO_PKG_DESCRIPTION"),
        }
    };
}
