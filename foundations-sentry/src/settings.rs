use foundations::settings::settings;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

/// Sentry hook settings.
///
/// All rate limits are disabled by default.
#[settings]
pub struct SentrySettings {
    /// Maximum number of events that can be emitted per second, per fingerprint.
    ///
    /// Shorthand for a `per_fingerprint_rate_limit` of `max_events_per_second` events every
    /// second. Ignored if `per_fingerprint_rate_limit` is enabled.
    pub max_events_per_second: Option<NonZeroU32>,

    /// Rate limit applied to each event fingerprint separately.
    pub per_fingerprint_rate_limit: EventRateLimitSettings,

    /// Rate limit shared by all events.
    ///
    /// If `fatal_rate_limit` is enabled, fatal events are limited by it instead.
    pub global_rate_limit: EventRateLimitSettings,

    /// Rate limit shared by all fatal events (e.g. panics).
    ///
    /// This keeps a flood of non-fatal events from crowding out fatal ones. If disabled, fatal
    /// events are limited by `global_rate_limit`.
    pub fatal_rate_limit: EventRateLimitSettings,

    /// Maximum number of fingerprints tracked by the per-fingerprint rate limit.
    ///
    /// Once reached, events with fingerprints that aren't tracked yet are not limited per
    /// fingerprint, but they are still subject to the global and fatal rate limits. `0` removes
    /// the cap.
    #[serde(default = "SentrySettings::default_max_tracked_fingerprints")]
    pub max_tracked_fingerprints: usize,
}

impl SentrySettings {
    fn default_max_tracked_fingerprints() -> usize {
        10_000
    }

    pub(crate) fn effective_per_fingerprint_rate_limit(&self) -> Option<EventQuota> {
        self.per_fingerprint_rate_limit.quota().or_else(|| {
            self.max_events_per_second.map(|max_events| EventQuota {
                max_events,
                period: Duration::from_secs(1),
            })
        })
    }

    pub(crate) fn max_tracked_fingerprints(&self) -> Option<NonZeroUsize> {
        NonZeroUsize::new(self.max_tracked_fingerprints)
    }
}

/// Settings of a Sentry event rate limit.
///
/// Up to `max_events` events are allowed at once, and the allowance is replenished evenly over
/// `period_secs`: e.g. `max_events: 10` and `period_secs: 60` allow a burst of 10 events, followed
/// by one event every 6 seconds.
#[settings]
pub struct EventRateLimitSettings {
    /// Maximum number of events allowed per `period_secs`. `0` disables the rate limit.
    pub max_events: u32,

    /// The period of the rate limit, in seconds. `0` disables the rate limit.
    #[serde(default = "EventRateLimitSettings::default_period_secs")]
    pub period_secs: u64,
}

impl EventRateLimitSettings {
    fn default_period_secs() -> u64 {
        1
    }

    pub(crate) fn quota(&self) -> Option<EventQuota> {
        let max_events = NonZeroU32::new(self.max_events)?;
        if self.period_secs == 0 {
            return None;
        }

        Some(EventQuota {
            max_events,
            period: Duration::from_secs(self.period_secs),
        })
    }
}

/// A validated, enabled event rate limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EventQuota {
    pub(crate) max_events: NonZeroU32,
    pub(crate) period: Duration,
}
