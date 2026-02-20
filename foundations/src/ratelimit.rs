use std::{
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// A rate limiter using the Generic Cell Rate Algorithm (GCRA).
///
/// GCRA is effectively a "leaky bucket" or "token bucket" algorithm that tracks
/// a theoretical arrival time (TAT) for requests. Each request advances the TAT
/// by a fixed interval. If the TAT would exceed the current time plus a
/// tolerance window, the request is rate limited.
///
/// This approach is memory-efficient (only stores a single timestamp) and
/// provides smooth rate limiting without the burstiness that can occur with
/// fixed time windows.
pub struct RateLimiter<'a> {
    /// When the next request should arrive if all requests arrive in order with
    /// perfect spacing (aka TAT).
    arrival_time: AtomicU64,
    config: &'a RateLimiterConfig,
}

/// Config for a [`RateLimiter`].
pub struct RateLimiterConfig {
    /// Nanoseconds between each request.
    request_spacing_ns: u64,
    /// How many nanoseconds from the current time we can push the arrival_time.
    /// This defines how much burst we allow. This is `request_spacing_ns *
    /// (burst + 1)` to allow for the base rate plus extra burst capacity.
    tolerance_ns: u64,
}

impl RateLimiterConfig {
    /// Create a new [`RateLimiterConfig`] with the given rate and burst. A rate
    /// of `1.0` and burst of `3` will ratelimit to 1 rps and allow up to 3
    /// requests before limiting.
    pub const fn new(rate: f64, burst: u64) -> Self {
        const NANOS_PER_SECOND: f64 = 1_000_000_000.0;
        let request_spacing_ns = (NANOS_PER_SECOND / rate) as u64;

        Self {
            request_spacing_ns,
            tolerance_ns: request_spacing_ns * (burst + 1),
        }
    }
}

impl<'a> RateLimiter<'a> {
    /// Create a new [`RateLimiter`] with the given config.
    pub const fn new(config: &'a RateLimiterConfig) -> Self {
        Self {
            arrival_time: AtomicU64::new(0),
            config,
        }
    }

    /// [`RateLimiter::is_ratelimited`] returns `true` if the caller is
    /// ratelimited and `false` if not.
    pub fn is_ratelimited(&self) -> bool {
        self.ratelimited_at(now_ns())
    }

    fn ratelimited_at(&self, now: u64) -> bool {
        let result =
            self.arrival_time
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |arrival_time| {
                    let new_arrival_time = arrival_time.max(now) + self.config.request_spacing_ns;

                    if new_arrival_time > now + self.config.tolerance_ns {
                        // If the new arrival time is too far ahead, we don't
                        // update (`None`) which will cause us to be
                        // ratelimited.
                        None
                    } else {
                        // The new time is valid. Attempt to update it.
                        Some(new_arrival_time)
                    }
                });

        result.is_err()
    }
}

fn now_ns() -> u64 {
    EPOCH.elapsed().as_nanos() as u64
}

/// Conditionally executes a block of code based on a per-callsite rate limiter.
///
/// Each invocation of this macro creates a unique static rate limiter at the call site.
/// The code block will only execute if the rate limiter allows it (i.e., not rate limited).
///
/// # Arguments
///
/// * `rate` - The rate limit in requests per second (e.g., `1.0` for 1 request/second)
/// * `burst` - The burst capacity (number of requests allowed before rate limiting kicks in)
///
/// # Example
///
/// ```
/// use foundations::ratelimit;
///
/// // Allow 1 request per second with burst of 3
/// ratelimit!(rate=1.0, burst=3, {
///     println!("This will be rate limited");
/// });
/// ```
#[macro_export]
macro_rules! ratelimit {
    (rate=$rate:expr, burst=$burst:expr, { $($body:tt)* }) => {
        {
            static __RATELIMIT_CONFIG: $crate::RateLimiterConfig =
                $crate::RateLimiterConfig::new($rate, $burst);
            static __RATELIMITER: $crate::RateLimiter =
                $crate::RateLimiter::new(&__RATELIMIT_CONFIG);

            if !__RATELIMITER.is_ratelimited() {
                $($body)*
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    /// Executes a series of rate limiter tests in order.
    /// Each test is a tuple of (timestamp_ns, expected_result).
    /// Panics with the index of the first failing test.
    fn test_ratelimiter(limiter: RateLimiter, tests: &[(u64, bool)]) {
        for (i, (time_ns, expected)) in tests.iter().enumerate() {
            let result = limiter.ratelimited_at(*time_ns);
            assert_eq!(
                result, *expected,
                "Test index {} failed at t={}ns: expected {}, got {}",
                i, time_ns, expected, result
            );
        }
    }

    #[test]
    #[should_panic(expected = "Test index 1 failed")]
    fn test_ratelimiter_catches_failures() {
        // Verify the test helper itself catches mismatches
        let config = RateLimiterConfig::new(1.0, 0);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                (0, false),
                // This should fail - second request at t=0 will be limited
                (0, false),
            ],
        );
    }

    #[test]
    fn zero_burst_base_rate() {
        // With burst=0, only the base rate is allowed (no extra burst capacity)
        let config = RateLimiterConfig::new(1.0, 0);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                // First request allowed
                (0, false),
                // Second request at same time is limited
                (0, true),
                // After 1 second, next request allowed
                (NANOS_PER_SECOND, false),
                // Immediately after, limited again
                (NANOS_PER_SECOND, true),
            ],
        );
    }

    #[test]
    fn burst_capacity_and_refill() {
        // 1 request per second, extra burst of 2
        // Should allow 3 requests at t=0 (1 base + 2 burst)
        let config = RateLimiterConfig::new(1.0, 2);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                // Use up all burst at t=0
                (0, false),
                (0, false),
                (0, false),
                // Fourth request should be rate limited
                (0, true),
                // After 3 seconds, burst should be fully refilled
                (3 * NANOS_PER_SECOND, false),
                (3 * NANOS_PER_SECOND, false),
                (3 * NANOS_PER_SECOND, false),
                (3 * NANOS_PER_SECOND, true),
            ],
        );
    }

    #[test]
    fn tokens_refill_over_time() {
        // 1 request per second, extra burst of 1
        let config = RateLimiterConfig::new(1.0, 1);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                // Use up burst at t=0 (1 base + 1 extra = 2 total)
                (0, false),
                (0, false),
                (0, true),
                // After 1 second, we should have 1 token available
                (NANOS_PER_SECOND, false),
                (NANOS_PER_SECOND, true),
                // After 2 seconds from start, we should have another token
                (2 * NANOS_PER_SECOND, false),
            ],
        );
    }

    #[test]
    fn high_rate_limiter() {
        // 1000 requests per second, no extra burst
        let spacing_ns = NANOS_PER_SECOND / 1000; // 1ms between requests

        let config = RateLimiterConfig::new(1000.0, 0);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                // First request allowed
                (0, false),
                // Immediate second request should be limited
                (0, true),
                // After 1ms, should be allowed
                (spacing_ns, false),
            ],
        );
    }

    #[test]
    #[expect(clippy::erasing_op)]
    fn steady_rate_within_limit() {
        // 10 requests per second, no extra burst
        let spacing_ns = NANOS_PER_SECOND / 10; // 100ms between requests

        // Requests spaced exactly at the rate limit should all succeed
        let config = RateLimiterConfig::new(10.0, 0);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                (0 * spacing_ns, false),
                (1 * spacing_ns, false),
                (2 * spacing_ns, false),
                (3 * spacing_ns, false),
                (4 * spacing_ns, false),
                (5 * spacing_ns, false),
                (6 * spacing_ns, false),
                (7 * spacing_ns, false),
                (8 * spacing_ns, false),
                (9 * spacing_ns, false),
            ],
        );
    }

    #[test]
    fn fractional_rate() {
        // 0.5 requests per second = 1 request every 2 seconds
        let config = RateLimiterConfig::new(0.5, 0);
        test_ratelimiter(
            RateLimiter::new(&config),
            &[
                // First request allowed
                (0, false),
                // After 1 second, still limited
                (NANOS_PER_SECOND, true),
                // After 2 seconds, allowed
                (2 * NANOS_PER_SECOND, false),
            ],
        );
    }
}
