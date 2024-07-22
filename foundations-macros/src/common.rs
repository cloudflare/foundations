use darling::ast::NestedMeta;
use quote::ToTokens as _;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Ident, Token};

pub(crate) type Result<T> = std::result::Result<T, syn::Error>;

pub(crate) fn error<T>(spanned: &impl Spanned, msg: &'static str) -> Result<T> {
    Err(syn::Error::new(spanned.span(), msg))
}

pub(crate) fn parse_meta_list(input: &ParseStream) -> syn::Result<Vec<NestedMeta>> {
    let list = Punctuated::<NestedMeta, Token![,]>::parse_terminated(input)?
        .into_iter()
        .collect();

    Ok(list)
}

pub(crate) fn parse_optional_trailing_meta_list(
    input: &ParseStream,
) -> syn::Result<Vec<NestedMeta>> {
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;

        parse_meta_list(input)
    } else {
        Ok(Default::default())
    }
}

pub(crate) fn parse_attr_value<T>(attr: Attribute) -> Result<T>
where
    T: Parse,
{
    let parser = |input: ParseStream| {
        let _name = input.parse::<Ident>()?;
        let _equal_token = input.parse::<Token![=]>()?;
        input.parse::<T>()
    };

    parser.parse2(attr.meta.to_token_stream())
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

    macro_rules! parse_attr {
        ( $($t:tt)* ) => {{
            let attr: syn::Attribute = parse_quote! { $($t)* };
            let args_tokens: proc_macro2::TokenStream = attr.parse_args().unwrap_or_default();

            syn::parse2(args_tokens.into()).unwrap()
        }};
    }

    pub(crate) use code_str;
    pub(crate) use parse_attr;
}
