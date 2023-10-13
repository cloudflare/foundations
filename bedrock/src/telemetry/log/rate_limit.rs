use crate::telemetry::settings::LoggingSettings;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use slog::{Drain, Never, OwnedKVList, Record};

pub(crate) struct RateLimitingDrain<D: Drain<Err = Never>> {
    inner: D,
    rate_limiter: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
}

impl<D: Drain<Err = Never>> RateLimitingDrain<D> {
    pub(crate) fn new(inner: D, settings: &LoggingSettings) -> Self {
        let rate_limiter = if settings.rate_limit.enabled {
            settings
                .rate_limit
                .max_events_per_second
                .try_into()
                .ok()
                .map(|r| RateLimiter::direct(Quota::per_second(r)))
        } else {
            None
        };

        Self {
            inner,
            rate_limiter,
        }
    }
}

impl<D: Drain<Err = Never>> Drain for RateLimitingDrain<D> {
    type Ok = ();
    type Err = D::Err;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let should_log = self
            .rate_limiter
            .as_ref()
            .map(|r| r.check().is_ok())
            .unwrap_or(true);

        if should_log {
            self.inner.log(record, values).map(|_| ())
        } else {
            Ok(())
        }
    }
}
