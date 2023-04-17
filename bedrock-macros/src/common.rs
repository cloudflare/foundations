use syn::spanned::Spanned;

pub(crate) type Result<T> = std::result::Result<T, syn::Error>;

pub(crate) fn error<T>(spanned: &impl Spanned, msg: &'static str) -> Result<T> {
    Err(syn::Error::new(spanned.span(), msg))
}

#[cfg(test)]
pub(crate) mod test_utils {
    macro_rules! code_str {
        ($($t:tt)*) => {{
            // NOTE: parse-compile to discard formating
            let parsed: proc_macro2::TokenStream = parse_quote!{ $($t)* };

            parsed.to_string()
        }};
    }

    pub(crate) use code_str;
}
