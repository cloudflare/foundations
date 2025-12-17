//! Sentry hook implementation for tracking sentry events.

use super::FatalErrorRegistry;
use super::_private::{DefaultBuilderState, HasRegistry, NeedsRegistry};

#[cfg(feature = "metrics")]
use crate::alerts::_private::DefaultRegistry;

/// Returns a builder for configuring and installing the sentry hook. The sentry
/// hook is installed by modifying a provided [`sentry_core::ClientOptions`].
///
/// When the `metrics` feature is enabled, the `foundations::metrics` registry
/// is used [`SentryHookBuilder::install()`] can be called immediately. When
/// `metrics` is disabled, you must call [`SentryHookBuilder::with_registry()`]
/// before `.install()`.
///
/// See the module-level docs for more information: [`crate::alerts`].
pub fn sentry_hook() -> SentryHookBuilder<DefaultBuilderState> {
    SentryHookBuilder {
        state: Default::default(),
    }
}

/// Builder for configuring the sentry hook.
///
/// This builder uses the typestate pattern to ensure at compile time that a
/// registry is available before `.install()` can be called. When the `metrics`
/// feature is enabled, a default registry is provided automatically.
#[must_use = "A SentryHookBuilder should be installed with .install()"]
pub struct SentryHookBuilder<State> {
    state: State,
}

impl SentryHookBuilder<NeedsRegistry> {
    /// Provide a custom metrics registry for recording fatal error metrics.
    ///
    /// This is required when the `metrics` feature is disabled.
    pub fn with_registry<R>(self, registry: R) -> SentryHookBuilder<HasRegistry<R>>
    where
        R: FatalErrorRegistry + Send + Sync + 'static,
    {
        SentryHookBuilder {
            state: HasRegistry { registry },
        }
    }
}

#[cfg(feature = "metrics")]
impl SentryHookBuilder<HasRegistry<DefaultRegistry>> {
    /// Provide a custom metrics registry for recording fatal error metrics.
    ///
    /// This overrides the default `foundations::metrics` registry.
    pub fn with_registry<R>(self, registry: R) -> SentryHookBuilder<HasRegistry<R>>
    where
        R: FatalErrorRegistry + Send + Sync + 'static,
    {
        SentryHookBuilder {
            state: HasRegistry { registry },
        }
    }
}

impl<R: FatalErrorRegistry + Send + Sync + 'static> SentryHookBuilder<HasRegistry<R>> {
    /// Install the sentry hook on the provided client options.
    ///
    /// This installs a `before_send` hook that increments `sentry_events_total`.
    /// If a previous `before_send` hook exists, it will be called after incrementing
    /// the metric.
    pub fn install(self, options: &mut sentry_core::ClientOptions) {
        use std::sync::Arc;

        let registry = Arc::new(self.state.registry);
        let previous = options.before_send.take();

        options.before_send = Some(Arc::new(move |event| {
            registry.inc_sentry_events_total(event.level.into(), 1);

            // Call previous hook if any
            if let Some(ref prev) = previous {
                prev(event)
            } else {
                Some(event)
            }
        }));
    }
}
