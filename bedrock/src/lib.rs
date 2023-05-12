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
//! - **settings**: Enables serializable documented settings functionality.
//! - **telemetry**: Enables all the telemetry-related features (**logging**, **tracing**).
//! - **logging**: Enables logging functionality.
//! - **tracing**: Enables distributed tracing functionality.
//! - **testing**: Enables testing-related functionality.
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

/// A macro that implements the [`Settings`] trait for a structure or an enum
/// and turns Rust doc comments into serializable documentation.
///
/// The macro automatically implements [`serde::Serialize`], [`serde::Deserialize`], [`Clone`],
/// [`Default`] and [`std::fmt::Debug`] traits for the type. Certain automatic trait implementations
/// can be disabled via macro arguments (see examples below).
///
/// # Example
/// ```
/// use bedrock::settings::to_yaml_string;
///
/// #[bedrock::settings]
/// struct NestedStruct {
///     /// A field.
///     a: usize,
///     /// multi-line
///     /// doc comment
///     b: u32,
///     // no doc comment at all
///     c: u32,
/// }
///
/// #[bedrock::settings]
/// struct SimpleStruct {
///     /// The documentation of NestedStruct
///     /// will be added to the keys of `inner`
///     inner: NestedStruct,
///     /// Another important field
///     x: u32,
/// }
///
/// let config_yaml = to_yaml_string(&SimpleStruct::default()).unwrap();
///
/// println!("{config_yaml}");
/// ```
///
/// will print the following YAML:
/// ```yml
/// ---
/// # The documentation of NestedStruct
/// # will be added to the keys of `inner`
/// inner:
///   # A field.
///   a: 0
///   # multi-line
///   # doc comment
///   b: 0
///   c: 0
/// # Another important field
/// x: 0
/// ```
///
/// # [`Default`] implementation
///
/// The macro provides a [`Default`] implementation that takes in consideration
/// `#[serde(default = ...)]` attributes:
///
/// ```
/// use bedrock::settings::Settings;
///
/// #[bedrock::settings]
/// struct SimpleStruct {
///     a: usize,
///     #[serde(default = "SimpleStruct::default_b")]
///     b: u32,
/// }
///
/// impl SimpleStruct {
///     fn default_b() -> u32 {
///         42
///     }
/// }
///
/// let inst = SimpleStruct::default();
///
/// assert_eq!(inst.a, 0);
/// assert_eq!(inst.b, 42);
/// ```
///
/// # Custom [`Default`] implementation
///
/// Sometimes it's desirable to manually implement [`Default`], e.g. for enums where default value
/// is not a unit variant, in this case usage of `#[default]` attribute will fail to compile:
///
/// ```compile_fail
/// #[bedrock::settings]
/// enum WonderfulVariants {
///     #[default]
///     VariantWithString(String),
///     UnitVariant
/// }
/// ```
///
/// The macro can be instructed to not generate [`Default`] implementation to workaround that:
///
/// ```
/// #[bedrock::settings(impl_default = false)]
/// #[derive(PartialEq)]
/// enum WonderfulVariants {
///     VariantWithString(String),
///     UnitVariant
/// }
///
/// impl Default for WonderfulVariants {
///     fn default() -> Self {
///         Self::VariantWithString("Hi there".into())
///     }
/// }
///
/// assert_eq!(
///     WonderfulVariants::default(),
///     WonderfulVariants::VariantWithString("Hi there".into())
/// );
/// ```
///
/// # Custom [`std::fmt::Debug`] implementation
///
/// One may want to have custom formatting code for a structure or enum. In this case the macro
/// can be instructed to not automatically generate derive implementation:
///
/// ```
/// use std::fmt;
///
/// #[bedrock::settings(impl_debug = false)]
/// struct Hello {
///     who: String
/// }
///
/// impl fmt::Debug for Hello {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "Hello {}", self.who)
///     }
/// }
/// ```
///
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `bedrock` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// mod reexport {
///     pub use bedrock::*;
/// }
///
/// #[reexport::settings(crate_path = "reexport")]
/// struct Foo {
///     bar: String
/// }
/// ```
///
/// [`Settings`]: crate::settings::Settings
#[cfg(feature = "settings")]
pub use bedrock_macros::settings;

#[doc(hidden)]
pub mod reexports_for_macros {
    #[cfg(feature = "tracing")]
    pub use rustracing;
    #[cfg(feature = "settings")]
    pub use serde;
    #[cfg(feature = "logging")]
    pub use slog;
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
