use crate::telemetry::settings::ActiveSamplingSettings;
use cf_rustracing::sampler::Sampler;
use cf_rustracing::span::CandidateSpan;
use cf_rustracing::{sampler::ProbabilisticSampler, Result};
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};

#[derive(Debug)]
pub(crate) struct RateLimitingProbabilisticSampler {
    inner: ProbabilisticSampler,
    rate_limiter: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
}

impl Default for RateLimitingProbabilisticSampler {
    fn default() -> Self {
        Self {
            inner: ProbabilisticSampler::new(0.0).unwrap(),
            rate_limiter: None,
        }
    }
}

/// A tracing sampler which also optionally rate limits the number of spans emitted
impl RateLimitingProbabilisticSampler {
    /// If `sampling_rate` is not in the range `0.0...1.0`,
    /// it will return an error with the kind `ErrorKind::InvalidInput`.
    pub(crate) fn new(settings: &ActiveSamplingSettings) -> Result<Self> {
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

        Ok(Self {
            inner: ProbabilisticSampler::new(settings.sampling_ratio)?,
            rate_limiter,
        })
    }
}

impl<T> Sampler<T> for RateLimitingProbabilisticSampler {
    fn is_sampled(&self, span: &CandidateSpan<T>) -> bool {
        if !self.inner.is_sampled(span) {
            return false;
        }

        self.rate_limiter
            .as_ref()
            .map(|r| r.check().is_ok())
            .unwrap_or(true)
    }
}
