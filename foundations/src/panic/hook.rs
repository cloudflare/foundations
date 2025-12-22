//! Panic hook implementation for tracking panics.

use std::panic::{self, PanicHookInfo};
use std::sync::OnceLock;

pub(crate) static HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

/// Install the panic hook.
///
/// Returns `true` if this is the first installation, `false` if the hook
/// was already installed (subsequent calls are no-ops).
///
/// See the module-level docs for more information: [`crate::panic`]
pub fn install_hook() -> bool {
    let first_install = HOOK_INSTALLED.set(()).is_ok();
    if !first_install {
        return false;
    }

    let previous = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        super::metrics::panics::total().inc();

        log_panic(panic_info);
        previous(panic_info);
    }));

    true
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
