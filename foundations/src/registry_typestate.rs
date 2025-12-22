//! Typestate types for builder patterns that optionally require a registry.
//!
//! Used by `panic` and `sentry` hook builders to enforce at compile time
//! that a registry is provided when the `metrics` feature is disabled.

/// Typestate indicating a registry must be provided before the builder can be used.
#[derive(Default)]
#[doc(hidden)]
pub struct NeedsRegistry(());

/// Typestate indicating a registry has been provided.
#[doc(hidden)]
pub struct HasRegistry<R> {
    pub(crate) registry: R,
}

/// The default registry implementation using foundations metrics.
#[cfg(feature = "metrics")]
#[doc(hidden)]
pub struct DefaultRegistry(());

#[cfg(feature = "metrics")]
#[doc(hidden)]
impl Default for HasRegistry<DefaultRegistry> {
    fn default() -> Self {
        Self {
            registry: DefaultRegistry(()),
        }
    }
}

#[cfg(feature = "metrics")]
#[doc(hidden)]
pub type DefaultBuilderState = HasRegistry<DefaultRegistry>;

#[cfg(not(feature = "metrics"))]
#[doc(hidden)]
pub type DefaultBuilderState = NeedsRegistry;
