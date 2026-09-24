//! Address-only stacktrace capture for server-side symbolication.

use sentry_core::protocol::{Event, Stacktrace};
use sentry_core::{ClientOptions, Integration};

/// Attaches an unresolved current-thread stacktrace to events without a stacktrace.
///
/// This respects [`ClientOptions::attach_stacktrace`] and preserves stacktraces already
/// attached to the event, any exception, or any thread. It captures instruction addresses
/// without loading debug information or resolving symbols.
///
/// Install this instead of [`sentry_backtrace::AttachStacktraceIntegration`], with
/// [`ClientOptions::default_integrations`] disabled. Server-side symbolication additionally
/// requires loaded-image metadata from `sentry_debug_images::DebugImagesIntegration` and
/// access to matching debug files.
///
/// # Panics
///
/// Client initialization panics if [`sentry_backtrace::AttachStacktraceIntegration`] is
/// also installed, since it could resolve symbols before this integration runs.
#[derive(Debug, Default)]
pub struct UnresolvedStacktraceIntegration;

impl Integration for UnresolvedStacktraceIntegration {
    fn name(&self) -> &'static str {
        "attach-unresolved-stacktrace"
    }

    fn setup(&self, options: &mut ClientOptions) {
        assert!(
            !options.integrations.iter().any(|integration| {
                integration
                    .as_ref()
                    .as_any()
                    .is::<sentry_backtrace::AttachStacktraceIntegration>()
            }),
            "UnresolvedStacktraceIntegration must replace sentry_backtrace::AttachStacktraceIntegration"
        );
    }

    fn process_event(
        &self,
        mut event: Event<'static>,
        options: &ClientOptions,
    ) -> Option<Event<'static>> {
        if !options.attach_stacktrace
            || event.stacktrace.is_some()
            || event
                .exception
                .iter()
                .any(|exception| exception.stacktrace.is_some())
            || event
                .threads
                .iter()
                .any(|thread| thread.stacktrace.is_some())
        {
            return Some(event);
        }
        if let Some(stacktrace) = unresolved_stacktrace() {
            let mut thread = sentry_backtrace::current_thread(false);
            thread.stacktrace = Some(stacktrace);
            event.threads.values.push(thread);
        }
        Some(event)
    }
}

pub(crate) fn unresolved_stacktrace() -> Option<Stacktrace> {
    sentry_backtrace::backtrace_to_stacktrace(&backtrace::Backtrace::new_unresolved())
}
