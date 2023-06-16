mod common;
mod info_metric;
mod metrics;
mod settings;
mod span_fn;
mod with_test_telemetry;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn info_metric(args: TokenStream, item: TokenStream) -> TokenStream {
    info_metric::expand(args, item)
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

#[proc_macro_attribute]
pub fn with_test_telemetry(args: TokenStream, item: TokenStream) -> TokenStream {
    with_test_telemetry::expand(args, item)
}
