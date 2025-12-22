//! Sentry hook implementation for tracking sentry events.

/// Install the sentry hook on the provided client options.
///
/// This installs a `before_send` hook that increments
/// `sentry_events_total`. If a previous `before_send` hook exists, it will
/// be called before incrementing the metric. Only unfiltered events are
/// counted.
///
/// See the module-level docs for more information: [`crate::sentry`].
pub fn install_hook(options: &mut sentry_core::ClientOptions) {
    use std::sync::Arc;

    let previous = options.before_send.take();

    options.before_send = Some(Arc::new(move |event| {
        let event = if let Some(prev) = &previous {
            prev(event)
        } else {
            Some(event)
        };

        if let Some(event) = &event {
            super::metrics::sentry_events::total(event.level).inc();
        }

        event
    }));
}
