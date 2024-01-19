use crate::common::{error, parse_optional_trailing_meta_list};
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_quote, Ident, ItemFn, Path, Signature};

const ERR_APPLIED_TO_NON_FN: &str = "`with_test_telemetry` macro can only be used on functions";
const ERR_NON_TEST_FN: &str = "`with_test_telemetry` can wrap only `test` or `tokio::test`";

#[derive(FromMeta)]
struct Options {
    #[darling(default = "Options::default_crate_path")]
    crate_path: Path,
}

impl Options {
    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }
}

struct Args {
    test_attr: Path,
    options: Options,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let test_attr = input.parse::<Path>()?;

        let is_tokio_test = test_attr.segments.len() == 2
            && test_attr.segments[0].ident == "tokio"
            && test_attr.segments[1].ident == "test";

        let is_rust_test = matches!(test_attr.get_ident(), Some(s) if s == "test");

        if !is_tokio_test && !is_rust_test {
            return error(&test_attr, ERR_NON_TEST_FN);
        }

        let meta_list = parse_optional_trailing_meta_list(&input)?;
        let options = Options::from_list(&meta_list)?;

        Ok(Self { test_attr, options })
    }
}

pub(crate) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as Args);

    let Ok(item_fn) = syn::parse(item) else {
        return syn::Error::new(Span::call_site(), ERR_APPLIED_TO_NON_FN)
            .to_compile_error()
            .into();
    };

    expand_from_parsed(args, item_fn).into()
}

fn expand_from_parsed(args: Args, item_fn: ItemFn) -> TokenStream2 {
    let ItemFn {
        attrs,
        vis,
        sig:
            Signature {
                output: return_type,
                inputs: params,
                unsafety,
                asyncness,
                constness,
                abi,
                ident,
                generics:
                    syn::Generics {
                        params: gen_params,
                        where_clause,
                        ..
                    },
                ..
            },
        block,
    } = item_fn;

    let crate_path = args.options.crate_path.clone();
    let test_attr = args.test_attr.clone();
    let inner_fn_ident = Ident::new(&format!("__{ident}"), ident.span());

    let inner_fn = quote!(
        #vis #constness #unsafety #asyncness #abi fn #inner_fn_ident<#gen_params>(#params) #return_type
        #where_clause
        #block
    );

    let wrapped_call = match asyncness {
        Some(_) => quote!(
            __ctx.clone().apply(async move { #inner_fn_ident(__ctx).await; }).await;
        ),
        None => quote!(
            let __scope = __ctx.scope();
            #inner_fn_ident(__ctx);
        ),
    };

    quote!(
        #(#attrs) *
        #[#test_attr]
        #vis #constness #unsafety #asyncness #abi fn #ident<#gen_params>() #return_type
        #where_clause
        {
            #inner_fn

            let __ctx = #crate_path::telemetry::TelemetryContext::test();

            #wrapped_call
        }
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use syn::parse_quote;

    #[test]
    fn expand_tokio_test() {
        let args = parse_attr! {
            #[with_test_telemetry(tokio::test)]
        };

        let item_fn = parse_quote! {
            async fn some_test(ctx: TestTelemetryContext) {
                assert!(false);
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            #[tokio::test]
            async fn some_test<>() {
                async fn __some_test<>(ctx: TestTelemetryContext) {
                    assert!(false);
                }

                let __ctx = ::foundations::telemetry::TelemetryContext::test();

                __ctx.clone().apply(async move { __some_test(__ctx).await; }).await;
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_rust_test() {
        let args = parse_attr! {
            #[with_test_telemetry(test)]
        };

        let item_fn = parse_quote! {
            fn some_test(ctx: TestTelemetryContext) {
                assert!(false);
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            #[test]
            fn some_test<>() {
                fn __some_test<>(ctx: TestTelemetryContext) {
                    assert!(false);
                }

                let __ctx = ::foundations::telemetry::TelemetryContext::test();
                let __scope = __ctx.scope();

                __some_test(__ctx);
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_with_crate_path() {
        let args = parse_attr! {
            #[with_test_telemetry(test, crate_path = "::foo::bar")]
        };

        let item_fn = parse_quote! {
            fn some_test(ctx: TestTelemetryContext) {
                assert!(false);
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            #[test]
            fn some_test<>() {
                fn __some_test<>(ctx: TestTelemetryContext) {
                    assert!(false);
                }

                let __ctx = ::foo::bar::telemetry::TelemetryContext::test();
                let __scope = __ctx.scope();

                __some_test(__ctx);
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    #[should_panic]
    fn non_test_fn_attr() {
        let _args: Args = parse_attr! {
            #[with_test_telemetry(foobar)]
        };
    }
}
