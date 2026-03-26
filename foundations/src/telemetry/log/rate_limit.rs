use crate::ratelimit::{DirectRateLimiter, Quota, StaticQuantaClock};
use crate::telemetry::settings::RateLimitingSettings;
use slog::{Drain, OwnedKVList, Record};
use std::num::NonZeroU32;

pub(crate) struct RateLimitingDrain<D> {
    inner: D,
    rate_limiter: Option<DirectRateLimiter>,
}

impl<D: Drain> RateLimitingDrain<D> {
    pub(crate) fn new(inner: D, settings: &RateLimitingSettings) -> Self {
        let rate_limiter = if settings.enabled
            && let Some(rate) = NonZeroU32::new(settings.max_events_per_second)
        {
            Some(DirectRateLimiter::direct_with_clock(
                Quota::per_second(rate),
                StaticQuantaClock::default(),
            ))
        } else {
            None
        };

        Self {
            inner,
            rate_limiter,
        }
    }
}

impl<D: Drain> Drain for RateLimitingDrain<D> {
    type Ok = ();
    type Err = D::Err;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        if let Some(limiter) = &self.rate_limiter
            && limiter.check().is_err()
        {
            return Ok(());
        }

        self.inner.log(record, values).map(|_| ())
    }

    #[inline]
    fn is_enabled(&self, level: slog::Level) -> bool {
        Drain::is_enabled(&self.inner, level)
    }

    #[inline]
    fn flush(&self) -> Result<(), slog::FlushError> {
        Drain::flush(&self.inner)
    }
}
