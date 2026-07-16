//! Reporting of non-fatal diagnostics produced while collecting metrics.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::sync::OnceLock;

type CollectErrorHook = Box<dyn for<'a> Fn(fmt::Arguments<'a>) + Send + Sync>;

static COLLECT_ERROR_HOOK: OnceLock<CollectErrorHook> = OnceLock::new();

/// Error returned by [`set_collect_error_hook`] when a hook is already installed.
#[derive(Debug)]
pub struct CollectErrorHookAlreadySet(());

impl fmt::Display for CollectErrorHookAlreadySet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a metric collection error hook is already installed")
    }
}

impl Error for CollectErrorHookAlreadySet {}

/// Routes non-fatal metric-collection diagnostics through a custom hook.
///
/// Collecting metrics never fails: label sets or metric groups that cannot be
/// encoded are skipped and a non-fatal diagnostic is emitted instead. By default
/// these diagnostics are written to standard error.
///
/// `foundations-metrics` is the low-level layer of the metrics stack and
/// deliberately depends on no logging framework, keeping logging decoupled from
/// metrics and avoiding a dependency cycle with higher-level crates. Instead of
/// calling into a logger directly, it exposes this seam: install a hook to route
/// collection diagnostics into your logging pipeline (typically at warning
/// level). The hook applies process-wide and can only be set once.
///
/// ```no_run
/// foundations_metrics::set_collect_error_hook(|args| {
///     // Forward to your logging framework, e.g. `log`, `tracing`, or `slog`.
///     eprintln!("{args}");
/// })
/// .expect("collect error hook installed once during start-up");
/// ```
pub fn set_collect_error_hook(
    hook: impl for<'a> Fn(fmt::Arguments<'a>) + Send + Sync + 'static,
) -> Result<(), CollectErrorHookAlreadySet> {
    COLLECT_ERROR_HOOK
        .set(Box::new(hook))
        .map_err(|_| CollectErrorHookAlreadySet(()))
}

/// Emits a non-fatal collection diagnostic through the installed hook, falling
/// back to standard error when no hook is set.
pub(crate) fn report_collect_error(args: fmt::Arguments<'_>) {
    match COLLECT_ERROR_HOOK.get() {
        Some(hook) => hook(args),
        None => {
            let _ = writeln!(io::stderr().lock(), "{args}");
        }
    }
}
