//! Sentry hook implementation for tracking sentry events and rate-limiting them.

use super::SentrySettings;
use governor::{Quota, RateLimiter};
use std::borrow::Cow;
use std::num::NonZeroU32;
use std::sync::Arc;

type Fingerprint = Cow<'static, str>;

/// Clean up keys in the sentry rate limiter once every 20 minutes
/// (3 times per hour), with no burst allowed.
const SENTRY_LIMITER_CLEANUP_QUOTA: Quota =
    Quota::per_hour(NonZeroU32::new(3).unwrap()).allow_burst(NonZeroU32::new(1).unwrap());

/// Install the sentry hook on the provided client options with default settings.
///
/// **Deprecated**, use [`install_hook_with_settings`] instead to pass
/// settings explicitly.
#[deprecated = "Replaced by install_hook_with_settings."]
pub fn install_hook(options: &mut sentry_core::ClientOptions) {
    install_hook_with_settings(options, &Default::default());
}

/// Install the sentry hook on the provided client options.
///
/// This installs a `before_send` hook that increments `sentry_events_total`
/// and performs rate limiting, if configured. If a previous `before_send`
/// hook exists, it will be called after rate limiting has been applied.
/// Only unfiltered events are counted.
///
/// See the module-level docs for more information: [`crate::sentry`].
pub fn install_hook_with_settings(
    options: &mut sentry_core::ClientOptions,
    settings: &SentrySettings,
) {
    let rate_limiter = settings
        .max_events_per_second
        .map(|rl| RateLimiter::<Fingerprint, _, _>::dashmap(Quota::per_second(rl)));

    let previous = options.before_send.take();

    options.before_send = Some(Arc::new(move |mut event| {
        if let Some(limiter) = &rate_limiter {
            crate::ratelimit!(SENTRY_LIMITER_CLEANUP_QUOTA; limiter.retain_recent());

            let fp = extract_fingerprint(&event);
            if limiter.check_key(&fp).is_err() {
                return None;
            }
        }

        if let Some(prev) = &previous {
            event = prev(event)?;
        }

        super::metrics::sentry::events_total(event.level).inc();

        Some(event)
    }));
}

/// Derive a fingerprint for a sentry event to perform rate limiting.
///
/// We check for the following event attributes, in order:
/// 1. Explicit fingerprint (if set and not defaulted)
/// 2. Event message
/// 3. First exception value/type
/// 4. Fallback: event level name
fn extract_fingerprint(event: &sentry_core::protocol::Event<'static>) -> Fingerprint {
    use sentry_core::protocol::Level;

    // Try the explicitly-specified fingerprint first, but only if its not defaulted
    let explicit_fp = &event.fingerprint;
    if !explicit_fp.is_empty() && !is_sentry_default_fingerprint(explicit_fp) {
        if let [fp] = explicit_fp.as_ref() {
            // Just clone if the explicit fingerprint is a single element
            return fp.clone();
        }
        return explicit_fp.join("::").into();
    }

    // Try the event message, if there is one
    if let Some(msg) = &event.message {
        return msg.clone().into();
    }

    // Try the first attached exception, if there is one
    if let Some(exc) = event.exception.first() {
        if let Some(val) = &exc.value {
            return val.clone().into();
        }
        if !exc.ty.is_empty() {
            return exc.ty.clone().into();
        }
    }

    // Finally, fall back to the event level
    Cow::Borrowed(match event.level {
        Level::Debug => "level::debug",
        Level::Info => "level::info",
        Level::Warning => "level::warning",
        Level::Error => "level::error",
        Level::Fatal => "level::fatal",
    })
}

// Adapted from https://github.com/getsentry/sentry-rust/blob/0.47.0/sentry-types/src/protocol/v7.rs#L1619
fn is_sentry_default_fingerprint(fp: &[Cow<'_, str>]) -> bool {
    if let [fp] = fp {
        return matches!(fp.as_ref(), "{{ default }}" | "{{default}}");
    }
    false
}
