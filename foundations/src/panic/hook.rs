//! Panic hook implementation for tracking panics.

use std::panic::{self, PanicHookInfo};
use std::sync::OnceLock;

#[cfg(feature = "metrics")]
use super::DefaultRegistry;
use super::{DefaultBuilderState, HasRegistry, NeedsRegistry, PanicsMetricsRegistry};

pub(crate) static HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

/// Returns a builder for configuring and installing the panic hook.
///
/// When the `metrics` feature is enabled, a default registry is provided and
/// [`PanicHookBuilder::init()`] can be called immediately. When `metrics` is
/// disabled, you must call [`PanicHookBuilder::with_registry()`] before `.init()`.
///
/// See the module-level docs for more information: [`crate::panic`]
pub fn hook() -> PanicHookBuilder<DefaultBuilderState> {
    PanicHookBuilder {
        state: Default::default(),
    }
}

/// Builder for configuring the panic hook.
///
/// This builder uses the typestate pattern to ensure at compile time that a
/// registry is available before [`PanicHookBuilder::init()`] can be called.
/// When the `metrics` feature is enabled, `foundations::metrics` is used.
#[must_use = "A PanicHookBuilder should be installed with .init()"]
pub struct PanicHookBuilder<State> {
    pub(super) state: State,
}

impl PanicHookBuilder<NeedsRegistry> {
    /// Provide a custom metrics registry for recording panic metrics.
    ///
    /// This is required when the `metrics` feature is disabled.
    pub fn with_registry<R>(self, registry: R) -> PanicHookBuilder<HasRegistry<R>>
    where
        R: PanicsMetricsRegistry + 'static,
    {
        PanicHookBuilder {
            state: HasRegistry { registry },
        }
    }
}

/// When `metrics` feature is enabled, allow overriding the default registry.
#[cfg(feature = "metrics")]
impl PanicHookBuilder<HasRegistry<DefaultRegistry>> {
    /// Provide a custom metrics registry for recording panic metrics.
    ///
    /// This overrides the default foundations metrics registry.
    pub fn with_registry<R>(self, registry: R) -> PanicHookBuilder<HasRegistry<R>>
    where
        R: PanicsMetricsRegistry + 'static,
    {
        PanicHookBuilder {
            state: HasRegistry { registry },
        }
    }
}

impl<R: PanicsMetricsRegistry + 'static> PanicHookBuilder<HasRegistry<R>> {
    /// Install the panic hook.
    ///
    /// Returns `true` if this is the first installation, `false` if the hook
    /// was already installed (subsequent calls are no-ops).
    pub fn init(self) -> bool {
        let first_install = HOOK_INSTALLED.set(()).is_ok();
        if !first_install {
            return false;
        }

        let registry = self.state.registry;
        let previous = panic::take_hook();

        panic::set_hook(Box::new(move |panic_info| {
            registry.inc_panics_total(1);

            log_panic(panic_info);
            previous(panic_info);
        }));

        true
    }
}

/// Log the panic using foundations telemetry if initialized, otherwise print JSON to stderr.
fn log_panic(panic_info: &PanicHookInfo<'_>) {
    let location = panic_info.location();
    let payload = panic_payload_as_str(panic_info);

    // Use foundations logging if telemetry is initialized
    #[cfg(feature = "logging")]
    if crate::telemetry::is_initialized() {
        crate::telemetry::log::error!(
            "panic occurred";
            "payload" => payload,
            "location" => ?location,
        );
        return;
    }

    // Fallback to printing structured JSON to stderr
    let location_str = location
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());

    let json_output = serde_json::json!({
        "level": "error",
        "msg": "panic occurred",
        "payload": payload,
        "location": location_str
    });
    eprintln!("{}", json_output);
}

fn panic_payload_as_str<'a>(panic_info: &'a PanicHookInfo<'_>) -> &'a str {
    let payload = panic_info.payload();

    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "<non-string panic payload>"
    }
}
