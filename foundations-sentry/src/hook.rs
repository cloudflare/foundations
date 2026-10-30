//! Sentry hook implementation for tracking sentry events and rate-limiting them.

use crate::SentrySettings;
use crate::metrics;
use crate::rate_limit::{EventRateLimiter, RateLimitKind};
use std::sync::Arc;

/// Install the sentry hook on the provided client options.
///
/// This installs a `before_send` hook that increments `sentry_events_total`
/// and performs rate limiting, if configured. If a previous `before_send`
/// hook exists, it will be called after the per-fingerprint rate limit and
/// before the global and fatal rate limits, so that events it drops don't use
/// up their allowance. Only unfiltered events are counted. Events dropped by
/// the rate limits are counted by `sentry_events_rate_limited_total`.
///
/// See the [`foundations-sentry`](crate) crate-level docs for more information.
pub fn install_hook_with_settings(
    options: &mut sentry_core::ClientOptions,
    settings: &SentrySettings,
) {
    let rate_limiter = EventRateLimiter::from_settings(settings);

    let previous = options.before_send.take();

    options.before_send = Some(Arc::new(move |mut event| {
        if let Some(limiter) = &rate_limiter
            && let Err(limit) = limiter.check_fingerprint(&event)
        {
            record_rate_limited(&event, limit);
            return None;
        }

        if let Some(prev) = &previous {
            event = prev(event)?;
        }

        if let Some(limiter) = &rate_limiter
            && let Err(limit) = limiter.check_shared(&event)
        {
            record_rate_limited(&event, limit);
            return None;
        }

        metrics::sentry::events_total(event.level).inc();

        Some(event)
    }));
}

fn record_rate_limited(event: &sentry_core::protocol::Event<'_>, limit: RateLimitKind) {
    metrics::sentry::events_rate_limited_total(event.level, limit.as_str()).inc();
}
