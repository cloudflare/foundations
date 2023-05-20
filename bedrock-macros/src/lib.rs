mod common;
mod settings;
mod span_fn;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn settings(args: TokenStream, item: TokenStream) -> TokenStream {
    settings::expand(args, item)
}

#[proc_macro_attribute]
pub fn span_fn(args: TokenStream, item: TokenStream) -> TokenStream {
    span_fn::expand(args, item)
}
