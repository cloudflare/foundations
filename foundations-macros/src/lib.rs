mod common;
mod display;
mod info_metric;
mod low_cardinality_label;
mod metrics;
mod settings;
mod span_fn;
mod span_with_probe;
mod with_test_telemetry;

use proc_macro::TokenStream;

use crate::display::DisplayOptions;

/// Derives `Display` from literal `#[display("...")]` format strings.
///
/// This is inspired by `thiserror` annotations, but applies generically to
/// any struct where you want something cleaner than the derived `Debug`
/// representation.
///
/// Named fields use `{field}`; tuple fields use `{0}` or `{}`. Formatting traits
/// and explicit generic bounds are checked by Rust. No error chaining,
/// conversions, transparency, or additional formatting expressions are supported.
/// Use [`Error`] instead to also implement [`std::error::Error`].
/// Only `display` attributes are inspected; unrelated attributes are ignored.
///
/// ```
/// use foundations_macros::Display;
///
/// #[derive(Debug, Display)]
/// #[display("{value:.2}mm")]
/// struct Millimetres { value: f32 }
///
/// assert_eq!(Millimetres { value: 12.345 }.to_string(), "12.35mm");
/// ```
///
/// Unsupported `display` syntax is rejected:
///
/// ```compile_fail
/// #[derive(foundations_macros::Display)]
/// #[display(transparent)]
/// struct Unsupported(std::io::Error);
/// ```
///
/// ```compile_fail
/// #[derive(foundations_macros::Display)]
/// #[display("{}", .0)]
/// struct Unsupported(String);
/// ```
#[proc_macro_derive(Display, attributes(display))]
pub fn display(input: TokenStream) -> TokenStream {
    display::expand(input, DisplayOptions::None)
}

/// Derives `Display` and `std::error::Error` from literal `#[display("...")]`
/// format strings.
///
/// This is identical to the [`Display`] derive, but also implements the
/// standard `Error` trait so the type can be used as a root error without a
/// manual `impl std::error::Error for ...`.
///
/// ```
/// use foundations_macros::Error;
///
/// #[derive(Debug, Error)]
/// #[display("invalid value {value:?}")]
/// struct InvalidValue { value: String }
///
/// let err: Box<dyn std::error::Error> = Box::new(InvalidValue { value: "abc".into() });
/// assert_eq!(err.to_string(), "invalid value \"abc\"");
/// ```
#[proc_macro_derive(Error, attributes(display))]
pub fn error(input: TokenStream) -> TokenStream {
    display::expand(input, DisplayOptions::ImplError)
}

#[proc_macro_attribute]
pub fn info_metric(args: TokenStream, item: TokenStream) -> TokenStream {
    info_metric::expand(args, item)
}

#[proc_macro_derive(LowCardinalityLabel, attributes(low_cardinality_label))]
pub fn low_cardinality_label(input: TokenStream) -> TokenStream {
    low_cardinality_label::expand(input)
}

#[proc_macro_attribute]
pub fn metrics(args: TokenStream, item: TokenStream) -> TokenStream {
    metrics::expand(args, item)
}

#[proc_macro_attribute]
pub fn settings(args: TokenStream, item: TokenStream) -> TokenStream {
    settings::expand(args, item)
}

#[proc_macro_attribute]
pub fn span_fn(args: TokenStream, item: TokenStream) -> TokenStream {
    span_fn::expand(args, item)
}

/// Like `foundations::telemetry::tracing::span`, plus a per-span USDT probe
/// fired at span end.
///
/// Probes are only emitted on linux/x86_64; on any other platform the macro
/// degrades to a plain `foundations::telemetry::tracing::span` call.
///
/// The probe shows up to tracers as
/// `<binary>:<usdt_provider>:span_end__<sanitized span name>`, where
/// sanitization replaces `::` with `__` and any other non-alphanumeric
/// character with `_`.
///
/// Expands to a dedicated probe semaphore: a `static` in the `.probes` ELF
/// section that the tracer (like bpftrace) increments on attach. When the
/// semaphore is non-zero, the span start timestamp is recorded in the span
/// state (regardless of span sampling), and the per-span `probe_end` function's
/// address is stored alongside it. When the last clone of the span drops,
/// `probe_end` is called with the span duration in nanoseconds followed by the
/// `probe_args` values, executing the NOP whose address the `stapsdt` ELF note
/// publishes as the `span_end__<sanitized span name>` probe location. Tracers
/// see the duration as arg0 and each `probe_args` value after it.
///
/// # Example
///
/// ```rust,ignore
/// use foundations::telemetry::tracing::span_with_probe;
///
/// span_with_probe!("http::client::send_request", usdt_provider = "myapp", probe_args = [opcode as u64])
///      .into_context()
///      .apply(do_exchange())
///      .await
/// ```
///
/// Options:
/// - `crate_path = "..."` (defaults to `::foundations`)
/// - `usdt_provider = "..."` (defaults to the `FOUNDATIONS_USDT_PROVIDER`
///   environment variable at compile time — settable per project via `[env]`
///   in `.cargo/config.toml` — or `"foundations"` when unset); must be
///   non-empty and must not contain `:`
/// - `probe_args = [expr, ...]` (defaults to `[]`); up to 3 opaque `u64`
///   values passed to the tracer as the probe's arguments after the duration
///   (arg0), each evaluated only when the probe is armed
#[proc_macro]
pub fn span_with_probe(input: TokenStream) -> TokenStream {
    span_with_probe::expand(input)
}

#[proc_macro_attribute]
pub fn with_test_telemetry(args: TokenStream, item: TokenStream) -> TokenStream {
    with_test_telemetry::expand(args, item)
}
