mod common;
mod info_metric;
mod low_cardinality_label;
mod metrics;
mod settings;
mod span_fn;
mod span_with_probe;
mod with_test_telemetry;

use proc_macro::TokenStream;

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
