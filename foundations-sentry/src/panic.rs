//! Sentry panic integration that records panic events without flushing immediately.

use std::any::Any;
use std::panic::{self, PanicHookInfo};
use std::sync::Once;

use sentry_core::protocol::{Event, Exception, Level, Mechanism};
use sentry_core::{ClientOptions, Integration};

use crate::backtrace::unresolved_stacktrace;

/// A Sentry panic handler [`Integration`] that does not flush after each event.
///
/// This emits the same events as [`sentry_panic::PanicIntegration`] by using
/// its public event construction API, but avoids flushing the events individually
/// to reduce time spent in the panic hook.
#[derive(Debug, Default)]
pub struct NoFlushPanicIntegration {
    inner: sentry_panic::PanicIntegration,
}

impl NoFlushPanicIntegration {
    /// Creates a new no-flush panic integration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures panic stacktraces without resolving symbols or loading debug information.
    ///
    /// Symbolication remains enabled unless this method is called. Server-side
    /// symbolication requires loaded-image metadata from
    /// `sentry_debug_images::DebugImagesIntegration` and access to matching debug files.
    /// Previously installed panic hooks still run and may resolve their own stacktraces.
    #[must_use]
    pub fn with_unresolved_stacktraces(mut self) -> Self {
        self.inner = self.inner.add_extractor(|info| {
            Some(Event {
                exception: vec![Exception {
                    ty: "panic".into(),
                    mechanism: Some(Mechanism {
                        ty: "panic".into(),
                        handled: Some(false),
                        ..Default::default()
                    }),
                    value: Some(sentry_panic::message_from_panic_info(info).to_owned()),
                    stacktrace: unresolved_stacktrace(),
                    ..Default::default()
                }]
                .into(),
                level: Level::Fatal,
                ..Default::default()
            })
        });
        self
    }
}

static INIT: Once = Once::new();

impl Integration for NoFlushPanicIntegration {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn setup(&self, cfg: &mut ClientOptions) {
        // `cfg.integrations` is copied before `setup` is called, so we
        // can't remove an upstream integration ourselves.
        let upstream_integration: Option<&sentry_panic::PanicIntegration> = cfg
            .integrations
            .iter()
            .find_map(|i| <dyn Any>::downcast_ref(i));

        if let Some(integ) = upstream_integration {
            panic!(
                "Found an upstream `sentry_panic::PanicIntegration` while installing `NoFlushPanicIntegration`: {integ:?}. This defeats the purpose of NoFlushPanicIntegration and will cause duplicate events."
            );
        }

        INIT.call_once(|| {
            let next = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                panic_handler(info);
                next(info)
            }));
        });
    }
}

fn panic_handler(info: &PanicHookInfo) {
    sentry_core::with_integration(|integration: &NoFlushPanicIntegration, hub| {
        hub.capture_event(integration.inner.event_from_panic_info(info));
        // no `client.flush()`!
    });
}
