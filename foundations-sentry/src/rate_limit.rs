//! Rate limits for Sentry events.

use crate::settings::{EventQuota, SentrySettings};
use foundations::ratelimit::StaticQuantaClock;
use governor::clock::Clock;
use governor::middleware::NoOpMiddleware;
use governor::nanos::Nanos;
use governor::state::keyed::{DashMapStateStore, ShrinkableKeyedStateStore};
use governor::state::{InMemoryState, NotKeyed, StateStore};
use governor::{Quota, RateLimiter};
use sentry_core::Level;
use sentry_core::protocol::Event;
use std::borrow::Cow;
use std::hash::{BuildHasher, RandomState};
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

/// Clean up idle fingerprints once every 20 minutes (3 times per hour), with no burst allowed.
const PERIODIC_CLEANUP_QUOTA: Quota =
    Quota::per_hour(NonZeroU32::new(3).unwrap()).allow_burst(NonZeroU32::new(1).unwrap());

/// Clean up idle fingerprints at most once per second while the fingerprint cap is reached.
const FULL_CLEANUP_QUOTA: Quota = Quota::per_second(NonZeroU32::new(1).unwrap());

type DirectLimiter<C> =
    RateLimiter<NotKeyed, InMemoryState, C, NoOpMiddleware<<C as Clock>::Instant>>;

type FingerprintLimiter<C> =
    RateLimiter<u64, BoundedStateStore, C, NoOpMiddleware<<C as Clock>::Instant>>;

/// The rate limit that rejected an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateLimitKind {
    Fingerprint,
    Global,
    Fatal,
}

impl RateLimitKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fingerprint => "fingerprint",
            Self::Global => "global",
            Self::Fatal => "fatal",
        }
    }
}

/// Enforces the rate limits configured in [`SentrySettings`].
pub(crate) struct EventRateLimiter<C: Clock = StaticQuantaClock> {
    /// Fingerprints are tracked by their hash, so that tracking a fingerprint takes a small,
    /// fixed amount of memory. A collision only makes two fingerprints share a rate limit.
    fingerprint_hasher: RandomState,
    per_fingerprint: Option<FingerprintLimiter<C>>,
    max_tracked_fingerprints: Option<NonZeroUsize>,
    global: Option<DirectLimiter<C>>,
    fatal: Option<DirectLimiter<C>>,
    periodic_cleanup: DirectLimiter<C>,
    full_cleanup: DirectLimiter<C>,
}

impl EventRateLimiter {
    /// Returns `None` if no rate limit is enabled.
    pub(crate) fn from_settings(settings: &SentrySettings) -> Option<Self> {
        Self::with_clock(settings, StaticQuantaClock::default())
    }
}

impl<C: Clock + Clone> EventRateLimiter<C> {
    fn with_clock(settings: &SentrySettings, clock: C) -> Option<Self> {
        let max_tracked_fingerprints = settings.max_tracked_fingerprints();
        let per_fingerprint = settings
            .effective_per_fingerprint_rate_limit()
            .map(|quota| {
                RateLimiter::new(
                    to_governor_quota(quota),
                    BoundedStateStore::new(max_tracked_fingerprints),
                    clock.clone(),
                )
            });
        let direct = |quota: EventQuota| {
            RateLimiter::direct_with_clock(to_governor_quota(quota), clock.clone())
        };
        let global = settings.global_rate_limit.quota().map(direct);
        let fatal = settings.fatal_rate_limit.quota().map(direct);

        if per_fingerprint.is_none() && global.is_none() && fatal.is_none() {
            return None;
        }

        Some(Self {
            fingerprint_hasher: RandomState::new(),
            per_fingerprint,
            max_tracked_fingerprints,
            global,
            fatal,
            periodic_cleanup: RateLimiter::direct_with_clock(PERIODIC_CLEANUP_QUOTA, clock.clone()),
            full_cleanup: RateLimiter::direct_with_clock(FULL_CLEANUP_QUOTA, clock),
        })
    }

    /// Takes the allowance of `event` from the per-fingerprint rate limit, if enabled.
    ///
    /// This should be checked before [`Self::check_shared`], so that events of a fingerprint
    /// that is already rate limited don't use up the allowance of the shared rate limits.
    pub(crate) fn check_fingerprint(&self, event: &Event<'_>) -> Result<(), RateLimitKind> {
        if let Some(limiter) = &self.per_fingerprint {
            self.clean_up(limiter);

            let fingerprint = extract_fingerprint(event);
            let key = self.fingerprint_hasher.hash_one(fingerprint.as_ref());
            if limiter.check_key(&key).is_err() {
                return Err(RateLimitKind::Fingerprint);
            }
        }

        Ok(())
    }

    /// Takes the allowance of `event` from the shared (global or fatal) rate limit that applies
    /// to it, if enabled.
    pub(crate) fn check_shared(&self, event: &Event<'_>) -> Result<(), RateLimitKind> {
        let (shared, kind) = match (&self.fatal, event.level) {
            (Some(fatal), Level::Fatal) => (Some(fatal), RateLimitKind::Fatal),
            _ => (self.global.as_ref(), RateLimitKind::Global),
        };
        if let Some(limiter) = shared
            && limiter.check().is_err()
        {
            return Err(kind);
        }

        Ok(())
    }

    /// Checks `event` against every rate limit, in the order used by the hook.
    #[cfg(test)]
    fn check(&self, event: &Event<'_>) -> Result<(), RateLimitKind> {
        self.check_fingerprint(event)?;
        self.check_shared(event)
    }

    /// Forgets fingerprints whose rate limit is back to its initial state.
    fn clean_up(&self, limiter: &FingerprintLimiter<C>) {
        let is_full = self
            .max_tracked_fingerprints
            .is_some_and(|max| limiter.len() >= max.get());
        let is_due =
            (is_full && self.full_cleanup.check().is_ok()) || self.periodic_cleanup.check().is_ok();

        if is_due {
            limiter.retain_recent();
        }
    }

    #[cfg(test)]
    fn tracked_fingerprints(&self) -> usize {
        self.per_fingerprint.as_ref().map_or(0, RateLimiter::len)
    }
}

/// Allows `max_events` at once, replenished evenly over `period`.
fn to_governor_quota(quota: EventQuota) -> Quota {
    // The period can only be zero for absurdly high limits: replenish as fast as possible then.
    let replenish_one_per = (quota.period / quota.max_events.get()).max(Duration::from_nanos(1));

    Quota::with_period(replenish_one_per)
        .expect("the period is non-zero")
        .allow_burst(quota.max_events)
}

/// A state store that tracks up to a maximum number of keys.
///
/// Once full, keys that aren't tracked yet are treated as if they were seen for the first time,
/// and are not tracked. The number of keys can slightly exceed the maximum when new keys are
/// inserted concurrently.
struct BoundedStateStore {
    entries: DashMapStateStore<u64>,
    max_entries: Option<NonZeroUsize>,
}

impl BoundedStateStore {
    fn new(max_entries: Option<NonZeroUsize>) -> Self {
        Self {
            entries: Default::default(),
            max_entries,
        }
    }
}

impl StateStore for BoundedStateStore {
    type Key = u64;

    fn measure_and_replace<T, F, E>(&self, key: &u64, f: F) -> Result<T, E>
    where
        F: Fn(Option<Nanos>) -> Result<(T, Nanos), E>,
    {
        // Only untracked keys need the size of the map, which locks every shard.
        let is_full = || {
            self.max_entries
                .is_some_and(|max| self.entries.len() >= max.get())
        };

        if !self.entries.contains_key(key) && is_full() {
            return f(None).map(|(outcome, _)| outcome);
        }

        StateStore::measure_and_replace(&self.entries, key, f)
    }
}

impl ShrinkableKeyedStateStore<u64> for BoundedStateStore {
    fn retain_recent(&self, drop_below: Nanos) {
        ShrinkableKeyedStateStore::retain_recent(&self.entries, drop_below);
    }

    fn shrink_to_fit(&self) {
        ShrinkableKeyedStateStore::shrink_to_fit(&self.entries);
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Derive a fingerprint for a sentry event to perform rate limiting.
///
/// We check for the following event attributes, in order:
/// 1. Explicit fingerprint (if set and not defaulted)
/// 2. Event message
/// 3. First exception value/type
/// 4. Fallback: event level name
fn extract_fingerprint<'a>(event: &'a Event<'_>) -> Cow<'a, str> {
    // Try the explicitly-specified fingerprint first, but only if its not defaulted
    let explicit_fp = &event.fingerprint;
    if !explicit_fp.is_empty() && !is_sentry_default_fingerprint(explicit_fp) {
        if let [fp] = explicit_fp.as_ref() {
            return Cow::Borrowed(fp);
        }
        return explicit_fp.join("::").into();
    }

    // Try the event message, if there is one
    if let Some(msg) = &event.message {
        return Cow::Borrowed(msg);
    }

    // Try the first attached exception, if there is one
    if let Some(exc) = event.exception.first() {
        if let Some(val) = &exc.value {
            return Cow::Borrowed(val);
        }
        if !exc.ty.is_empty() {
            return Cow::Borrowed(&exc.ty);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EventRateLimitSettings;
    use governor::clock::FakeRelativeClock;
    use sentry_core::protocol::Exception;

    fn rate_limit(max_events: u32, period_secs: u64) -> EventRateLimitSettings {
        EventRateLimitSettings {
            max_events,
            period_secs,
        }
    }

    fn limiter(
        settings: SentrySettings,
    ) -> (EventRateLimiter<FakeRelativeClock>, FakeRelativeClock) {
        let clock = FakeRelativeClock::default();
        let limiter = EventRateLimiter::with_clock(&settings, clock.clone())
            .expect("a rate limit is enabled");
        (limiter, clock)
    }

    fn event(fingerprint: &str) -> Event<'static> {
        Event {
            fingerprint: Cow::Owned(vec![Cow::Owned(fingerprint.to_owned())]),
            ..Event::default()
        }
    }

    fn fatal_event(fingerprint: &str) -> Event<'static> {
        Event {
            level: Level::Fatal,
            ..event(fingerprint)
        }
    }

    #[test]
    fn no_limiter_when_no_rate_limit_is_enabled() {
        assert!(EventRateLimiter::from_settings(&SentrySettings::default()).is_none());

        let disabled = SentrySettings {
            per_fingerprint_rate_limit: rate_limit(0, 60),
            global_rate_limit: rate_limit(10, 0),
            ..SentrySettings::default()
        };
        assert!(EventRateLimiter::from_settings(&disabled).is_none());
    }

    #[test]
    fn per_fingerprint_rate_limit_allows_a_burst_then_replenishes_evenly() {
        let (limiter, clock) = limiter(SentrySettings {
            per_fingerprint_rate_limit: rate_limit(10, 60),
            ..SentrySettings::default()
        });

        for _ in 0..10 {
            assert_eq!(limiter.check(&event("a")), Ok(()));
        }
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
        assert_eq!(limiter.check(&event("b")), Ok(()));

        // One event is replenished every `60s / 10`.
        clock.advance(Duration::from_secs(5));
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
        clock.advance(Duration::from_secs(1));
        assert_eq!(limiter.check(&event("a")), Ok(()));
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
    }

    #[test]
    fn max_events_per_second_is_a_per_fingerprint_rate_limit() {
        let (limiter, clock) = limiter(SentrySettings {
            max_events_per_second: NonZeroU32::new(2),
            ..SentrySettings::default()
        });

        for _ in 0..2 {
            assert_eq!(limiter.check(&event("a")), Ok(()));
        }
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
        assert_eq!(limiter.check(&event("b")), Ok(()));

        clock.advance(Duration::from_millis(500));
        assert_eq!(limiter.check(&event("a")), Ok(()));
    }

    #[test]
    fn per_fingerprint_rate_limit_overrides_max_events_per_second() {
        let (limiter, _clock) = limiter(SentrySettings {
            max_events_per_second: NonZeroU32::new(100),
            per_fingerprint_rate_limit: rate_limit(1, 60),
            ..SentrySettings::default()
        });

        assert_eq!(limiter.check(&event("a")), Ok(()));
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
    }

    #[test]
    fn global_rate_limit_is_shared_by_all_fingerprints() {
        let (limiter, clock) = limiter(SentrySettings {
            global_rate_limit: rate_limit(3, 60),
            ..SentrySettings::default()
        });

        for fingerprint in ["a", "b", "c"] {
            assert_eq!(limiter.check(&event(fingerprint)), Ok(()));
        }
        assert_eq!(limiter.check(&event("d")), Err(RateLimitKind::Global));
        assert_eq!(limiter.check(&fatal_event("e")), Err(RateLimitKind::Global));

        clock.advance(Duration::from_secs(20));
        assert_eq!(limiter.check(&event("d")), Ok(()));
    }

    #[test]
    fn fatal_events_have_their_own_rate_limit_when_enabled() {
        let (limiter, _clock) = limiter(SentrySettings {
            global_rate_limit: rate_limit(2, 60),
            fatal_rate_limit: rate_limit(1, 60),
            ..SentrySettings::default()
        });

        for fingerprint in ["a", "b"] {
            assert_eq!(limiter.check(&event(fingerprint)), Ok(()));
        }
        assert_eq!(limiter.check(&event("c")), Err(RateLimitKind::Global));

        assert_eq!(limiter.check(&fatal_event("d")), Ok(()));
        assert_eq!(limiter.check(&fatal_event("e")), Err(RateLimitKind::Fatal));
    }

    #[test]
    fn fatal_rate_limit_does_not_limit_other_events() {
        let (limiter, _clock) = limiter(SentrySettings {
            fatal_rate_limit: rate_limit(1, 60),
            ..SentrySettings::default()
        });

        assert_eq!(limiter.check(&fatal_event("a")), Ok(()));
        assert_eq!(limiter.check(&fatal_event("b")), Err(RateLimitKind::Fatal));
        for fingerprint in ["c", "d", "e"] {
            assert_eq!(limiter.check(&event(fingerprint)), Ok(()));
        }
    }

    #[test]
    fn events_rejected_per_fingerprint_do_not_use_the_global_allowance() {
        let (limiter, _clock) = limiter(SentrySettings {
            per_fingerprint_rate_limit: rate_limit(1, 60),
            global_rate_limit: rate_limit(2, 60),
            ..SentrySettings::default()
        });

        assert_eq!(limiter.check(&event("a")), Ok(()));
        for _ in 0..10 {
            assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));
        }
        assert_eq!(limiter.check(&event("b")), Ok(()));
        assert_eq!(limiter.check(&event("c")), Err(RateLimitKind::Global));
    }

    #[test]
    fn untracked_fingerprints_are_not_limited_per_fingerprint_once_the_cap_is_reached() {
        let (limiter, _clock) = limiter(SentrySettings {
            per_fingerprint_rate_limit: rate_limit(1, 60),
            global_rate_limit: rate_limit(5, 60),
            max_tracked_fingerprints: 2,
            ..SentrySettings::default()
        });

        assert_eq!(limiter.check(&event("a")), Ok(()));
        assert_eq!(limiter.check(&event("b")), Ok(()));
        assert_eq!(limiter.tracked_fingerprints(), 2);

        // Tracked fingerprints are still limited.
        assert_eq!(limiter.check(&event("a")), Err(RateLimitKind::Fingerprint));

        // Untracked ones are only subject to the global rate limit.
        for _ in 0..3 {
            assert_eq!(limiter.check(&event("c")), Ok(()));
        }
        assert_eq!(limiter.check(&event("c")), Err(RateLimitKind::Global));
        assert_eq!(limiter.tracked_fingerprints(), 2);
    }

    #[test]
    fn idle_fingerprints_are_forgotten_once_the_cap_is_reached() {
        let (limiter, clock) = limiter(SentrySettings {
            per_fingerprint_rate_limit: rate_limit(1, 60),
            max_tracked_fingerprints: 2,
            ..SentrySettings::default()
        });

        assert_eq!(limiter.check(&event("a")), Ok(()));
        assert_eq!(limiter.check(&event("b")), Ok(()));
        assert_eq!(limiter.tracked_fingerprints(), 2);

        // Once "a" and "b" are idle, they make room for new fingerprints. `governor` only forgets
        // a key one replenish interval after its rate limit is back to its initial state.
        clock.advance(Duration::from_secs(120));
        assert_eq!(limiter.check(&event("c")), Ok(()));
        assert_eq!(limiter.tracked_fingerprints(), 1);
        assert_eq!(limiter.check(&event("c")), Err(RateLimitKind::Fingerprint));
    }

    #[test]
    fn fingerprints_are_not_capped_when_the_cap_is_zero() {
        let (limiter, _clock) = limiter(SentrySettings {
            per_fingerprint_rate_limit: rate_limit(1, 60),
            max_tracked_fingerprints: 0,
            ..SentrySettings::default()
        });

        for i in 0..100 {
            assert_eq!(limiter.check(&event(&i.to_string())), Ok(()));
        }
        assert_eq!(limiter.tracked_fingerprints(), 100);
        assert_eq!(limiter.check(&event("0")), Err(RateLimitKind::Fingerprint));
    }

    #[test]
    fn quota_replenishes_as_fast_as_possible_for_absurdly_high_limits() {
        let quota = to_governor_quota(EventQuota {
            max_events: NonZeroU32::MAX,
            period: Duration::from_secs(1),
        });

        assert_eq!(quota.replenish_interval(), Duration::from_nanos(1));
        assert_eq!(quota.burst_size(), NonZeroU32::MAX);
    }

    #[test]
    fn fingerprint_uses_the_first_available_attribute() {
        let mut event = Event {
            level: Level::Warning,
            ..Event::default()
        };
        assert_eq!(extract_fingerprint(&event), "level::warning");

        event.exception = vec![Exception {
            ty: "panic".to_owned(),
            ..Exception::default()
        }]
        .into();
        assert_eq!(extract_fingerprint(&event), "panic");

        event.exception.values[0].value = Some("exception value".to_owned());
        assert_eq!(extract_fingerprint(&event), "exception value");

        event.message = Some("message".to_owned());
        assert_eq!(extract_fingerprint(&event), "message");

        event.fingerprint = Cow::Owned(vec!["{{ default }}".into()]);
        assert_eq!(extract_fingerprint(&event), "message");

        event.fingerprint = Cow::Owned(vec!["msg".into(), "file.rs".into(), "42".into()]);
        assert_eq!(extract_fingerprint(&event), "msg::file.rs::42");

        event.fingerprint = Cow::Owned(vec!["explicit".into()]);
        assert_eq!(extract_fingerprint(&event), "explicit");
    }
}
