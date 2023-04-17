mod common;
mod settings;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn settings(args: TokenStream, item: TokenStream) -> TokenStream {
    settings::expand(args, item)
}
