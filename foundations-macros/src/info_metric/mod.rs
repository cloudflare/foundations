use crate::common::Result;
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, parse_quote, Attribute, AttributeArgs, Ident, LitStr, Path, Token, Type,
    Visibility,
};

mod parsing;

#[derive(FromMeta)]
struct MacroArgs {
    #[darling(default = "Self::default_crate_path")]
    crate_path: Path,
    #[darling(default)]
    name: Option<String>,
}

impl Default for MacroArgs {
    fn default() -> Self {
        Self {
            crate_path: Self::default_crate_path(),
            name: None,
        }
    }
}

impl MacroArgs {
    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }
}

struct Struct {
    attrs: StructAttrs,
    vis: Visibility,
    struct_token: Token![struct],
    ident: Ident,
    fields: Punctuated<Field, Token![,]>,
}

#[derive(Default)]
struct StructAttrs {
    cfg: Vec<Attribute>,
    doc: String,
}

struct Field {
    attrs: FieldAttrs,
    vis: Visibility,
    ident: Ident,
    colon_token: Token![:],
    ty: Type,
}

#[derive(Default)]
struct FieldAttrs {
    serde: Option<Attribute>,
    serde_as: Option<Attribute>,
}

pub(crate) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let mod_ = parse_macro_input!(item as Struct);
    let args = match MacroArgs::from_list(&parse_macro_input!(args as AttributeArgs)) {
        Ok(args) => args,
        Err(e) => return e.write_errors().into(),
    };

    expand_from_parsed(args, mod_)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn expand_from_parsed(args: MacroArgs, struct_: Struct) -> Result<proc_macro2::TokenStream> {
    let MacroArgs {
        crate_path: foundations,
        name: metric_name,
    } = args;

    let Struct {
        attrs: StructAttrs { cfg, doc },
        vis: struct_vis,
        struct_token,
        ident: struct_name,
        fields,
    } = struct_;

    let metric_name = metric_name.unwrap_or_else(|| to_snake_case(&struct_name.to_string()));
    let help = str::trim(&doc);
    let serde_with = quote! { #foundations::reexports_for_macros::serde_with };
    let serde_with_str = LitStr::new(&serde_with.to_string(), Span::call_site());

    let serde_as_attr = fields
        .iter()
        .any(|arg| arg.attrs.serde_as.is_some())
        .then(|| quote! { #[#serde_with::serde_as(crate = #serde_with_str)] });

    let serde = quote! { #foundations::reexports_for_macros::serde };
    let serde_str = LitStr::new(&serde.to_string(), Span::call_site());

    Ok(quote! {
        #(#cfg)*
        #[doc = #doc]
        #serde_as_attr
        #[derive(#serde::Serialize)]
        #[serde(crate = #serde_str)]
        #struct_vis #struct_token #struct_name {
            #fields
        }

        #(#cfg)*
        impl #foundations::telemetry::metrics::InfoMetric for #struct_name {
            const NAME: &'static str = #metric_name;
            const HELP: &'static str = #help;
        }
    })
}

impl ToTokens for Field {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Field {
            attrs: FieldAttrs { serde, serde_as },
            vis,
            ident,
            colon_token,
            ty,
        } = self;

        tokens.extend(quote! {
            #serde
            #serde_as
            #vis #ident #colon_token #ty
        });
    }
}

fn to_snake_case(input: &str) -> String {
    let mut snake = String::new();

    for (i, ch) in input.char_indices() {
        if i > 0 && ch.is_uppercase() {
            snake.push('_');
        }
        snake.push(ch.to_ascii_lowercase());
    }

    snake
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use syn::parse_quote;

    #[test]
    fn expand_empty() {
        let attr = parse_attr! {
            #[info_metric]
        };

        let src = parse_quote! {
            /// Some info metric
            struct SomeInfoMetric {}
        };

        let actual = expand_from_parsed(attr, src).unwrap().to_string();

        let expected = code_str! {
            #[doc = " Some info metric"]
            #[derive(::foundations::reexports_for_macros::serde::Serialize)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            struct SomeInfoMetric {}

            impl ::foundations::telemetry::metrics::InfoMetric for SomeInfoMetric {
                const NAME: &'static str = "some_info_metric";
                const HELP: &'static str = "Some info metric";
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_simple() {
        let attr = parse_attr! {
            #[info_metric(crate_path = "tarmac", name = "build_info")]
        };

        let src = parse_quote! {
            /// Build information
            struct BuildInformation {
                #[serde(rename = "version")]
                vers: &'static str,
                #[serde_as(as = "DisplayFromStr")]
                mode: Mode,
            }
        };

        let actual = expand_from_parsed(attr, src).unwrap().to_string();

        let expected = code_str! {
            #[doc = " Build information"]
            #[tarmac::reexports_for_macros::serde_with::serde_as(crate = "tarmac :: reexports_for_macros :: serde_with")]
            #[derive(tarmac::reexports_for_macros::serde::Serialize)]
            #[serde(crate = "tarmac :: reexports_for_macros :: serde")]
            struct BuildInformation {
                #[serde(rename = "version")]
                vers: &'static str,
                #[serde_as(as = "DisplayFromStr")]
                mode: Mode,
            }

            impl tarmac::telemetry::metrics::InfoMetric for BuildInformation {
                const NAME: &'static str = "build_info";
                const HELP: &'static str = "Build information";
            }
        };

        assert_eq!(actual, expected);
    }
}
