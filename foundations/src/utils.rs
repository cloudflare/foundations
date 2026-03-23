// NOTE: don't complain about unused macro for feature combinations that don't use it.
#[allow(unused_macros)]
macro_rules! feature_use {
    ( $cfg:meta, { $( $tokens:tt )+ } ) => {
        // NOTE: a trick to apply attribute to all the tokens in a group without
        // introducing a new block or scope: we apply the attribute
        // to a macro call that expand to those tokens instead.
        #[$cfg]
        $crate::utils::feature_use!(@tokens $($tokens)*);
    };

    ( @tokens $( $tokens:tt )* ) => { $( $tokens )* };
}

// NOTE: don't complain about unused macro for feature combinations that don't use it.
#[allow(unused_imports)]
pub(crate) use feature_use;

/// Applies a rate limit to the evaluation of an expression.
///
/// The macro takes two arguments, separated by a `;`. The first is the quota to use
/// for the ratelimit. This can either be a const expression evaluating to a
/// [`governor::Quota`], or a rate specifier like `200/s`, `10/m`, or `5/h`. The latter
/// three are equivalent to [`Quota`]'s `per_second`/`per_minute`/`per_hour` constructors.
///
/// The second argument is the expression to evaluate if the rate limit has not been
/// reached yet. The expression's result will be discarded.
///
/// # Examples
/// ```rust
/// # fn expensive_computation() -> u32 { 42 }
/// #
/// use foundations::telemetry::log;
/// use governor::Quota;
/// use std::num::NonZeroU32;
///
/// foundations::ratelimit!(10/s; println!("frequently failing operation failed!") );
///
/// // You can return data from the expression with an Option:
/// let mut output = None;
/// foundations::ratelimit!(1/h; output.insert(expensive_computation()) );
/// assert_eq!(output, Some(42));
///
/// // A quota expression allows customizing the burst size. By default,
/// // it is equivalent to the rate per time unit (i.e., 10/m yields a burst size of 10).
/// // Note: you could also reference a `const` declared somewhere else here.
/// foundations::ratelimit!(
///     Quota::per_hour(NonZeroU32::new(100).unwrap()).allow_burst(NonZeroU32::new(1).unwrap());
///     println!("this will be printed only once before the rate limit kicks in")
/// );
///
/// // Here the rate limit kicks in after the initial burst of 60 iterations:
/// let mut counter = 0;
/// for _ in 0..1000 {
///     foundations::ratelimit!(60/h; counter += 1);
/// }
/// assert_eq!(counter, 60);
/// ```
///
/// [`Quota`]: governor::Quota
#[cfg(feature = "ratelimit")]
#[macro_export]
#[doc(hidden)]
macro_rules! __ratelimit {
    ($limit:literal / s ; $expr:expr) => {
        $crate::__ratelimit!(
            $crate::reexports_for_macros::governor::Quota::per_second(
                ::std::num::NonZeroU32::new($limit).unwrap()
            );
            $expr
        )
    };

    ($limit:literal / m ; $expr:expr) => {
        $crate::__ratelimit!(
            $crate::reexports_for_macros::governor::Quota::per_minute(
                ::std::num::NonZeroU32::new($limit).unwrap()
            );
            $expr
        )
    };

    ($limit:literal / h ; $expr:expr) => {
        $crate::__ratelimit!(
            $crate::reexports_for_macros::governor::Quota::per_hour(
                ::std::num::NonZeroU32::new($limit).unwrap()
            );
            $expr
        )
    };

    ($quota:expr ; $expr:expr) => {{
        const QUOTA: $crate::reexports_for_macros::governor::Quota = $quota;
        static LIMITER: ::std::sync::LazyLock<$crate::reexports_for_macros::governor::DefaultDirectRateLimiter> =
            ::std::sync::LazyLock::new(|| $crate::reexports_for_macros::governor::RateLimiter::direct(QUOTA));
        if LIMITER.check().is_ok() {
            $expr;
        }
    }};
}

#[cfg(feature = "ratelimit")]
pub use __ratelimit as ratelimit;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ratelimit")]
    #[test]
    fn test_ratelimit() {
        use governor::Quota;
        use std::num::NonZeroU32;

        const CUSTOM_QUOTA: Quota =
            Quota::per_hour(NonZeroU32::new(60).unwrap()).allow_burst(NonZeroU32::new(20).unwrap());

        // Burst size is only 20 for this quota, despite the refill rate being 60/h
        let mut res_custom = 0;
        for _ in 0..200 {
            ratelimit!(CUSTOM_QUOTA; res_custom += 1);
        }

        assert_eq!(res_custom, 20);

        // Cells may refill as the loop executes already, so a value >20 is possible
        let mut res_sec = 0;
        for _ in 0..100 {
            ratelimit!(20/s; res_sec += 1);
        }

        assert!(res_sec >= 20);
        assert!(res_sec < 100);

        // This should execute exactly 3 times; we don't expect any cells to refill
        let mut res_minute = 1;
        for _ in 0..20 {
            ratelimit!(3/m; res_minute *= 2);
        }

        assert_eq!(res_minute, 1 << 3);

        let mut res_hour_a = 0;
        let mut res_hour_b = 0;

        for _ in 0..1000 {
            ratelimit!(100/h; {
                res_hour_a += 1;
                res_hour_b += 2;
            });
        }

        assert!(res_hour_a >= 100);
        assert!(res_hour_a < 1000);
        assert_eq!(res_hour_b, 2 * res_hour_a);
    }
}
