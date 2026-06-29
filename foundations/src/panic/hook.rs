//! Panic hook implementation for tracking panics.

use std::io::Write;
use std::panic::{self, PanicHookInfo};

/// Install the panic hook.
///
/// Returns `true` if this is the first installation, `false` if the hook
/// was already installed (subsequent calls are no-ops).
///
/// See the module-level docs for more information: [`crate::panic`]
pub fn install_hook() -> bool {
    use std::sync::Once;
    static INSTALL_HOOK_ONCE: Once = Once::new();

    let mut first_install = false;

    INSTALL_HOOK_ONCE.call_once(|| {
        let previous = panic::take_hook();

        panic::set_hook(Box::new(move |panic_info| {
            super::metrics::panics::total().inc();

            log_panic(panic_info);
            previous(panic_info);
        }));

        first_install = true;
    });

    first_install
}

/// Log the panic using foundations telemetry if initialized, otherwise print JSON to stderr.
fn log_panic(panic_info: &PanicHookInfo<'_>) {
    let location = panic_info.location();
    let payload = panic_payload_as_str(panic_info);

    // Use foundations logging if telemetry is initialized
    #[cfg(feature = "logging")]
    if crate::telemetry::is_initialized() {
        let _ = crate::telemetry::log::internal::log_to_drain(&slog::record!(
            slog::Level::Error,
            "", // tag
            &format_args!("panic occurred"),
            slog::b!("payload" => payload, "location" => ?location),
        ));
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
    // Grab stderr ourselves and ignore any write error to avoid double-panicking
    // (e.g. if stderr is closed) within the panic hook.
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{}", json_output);
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

#[cfg(all(test, feature = "logging"))]
mod logging_tests {
    use crate::service_info;
    use crate::telemetry::TelemetryConfig;
    use crate::telemetry::log::init::{LogHarness, build_log_with_drain, wrap_root_drain};
    use crate::telemetry::log::internal::LoggerWithKvNestingTracking;
    use crate::telemetry::settings::LoggingSettings;
    use slog::{Drain, OwnedKVList, Record};
    use std::sync::Arc;

    struct FailingDrain;
    impl Drain for FailingDrain {
        type Ok = ();
        type Err = &'static str;

        fn log(&self, _record: &Record, _values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
            Err("drain failed")
        }
    }

    #[tokio::test]
    async fn hook_swallows_drain_error() {
        let settings = LoggingSettings::default();
        let root_drain = wrap_root_drain(&settings, FailingDrain);
        let root_log = LoggerWithKvNestingTracking::new(build_log_with_drain(
            settings.verbosity,
            slog::o!(),
            Arc::clone(&root_drain),
        ));

        LogHarness::override_for_testing(LogHarness {
            root_log: Arc::new(parking_lot::RwLock::new(root_log)),
            root_drain,
            settings,
            log_scope_stack: Default::default(),
        })
        .unwrap();

        crate::telemetry::init(TelemetryConfig {
            service_info: &service_info!(),
            settings: &Default::default(),
            custom_server_routes: Default::default(),
        })
        .expect("telemetry is already initialized");

        super::install_hook();
        let _ = std::panic::catch_unwind(|| panic!("oh no! 😱"));
        // If we just used `log::error!(...)` in the hook above, the `Fuse` from
        // `build_log_with_drain` would convert the error from `FailingDrain` into
        // a panic inside the panic hook. That's a double panic and aborts the process,
        // causing the test to fail.
    }
}
