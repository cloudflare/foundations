use crate::common::{error, parse_optional_trailing_meta_list};
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::{parse_quote, Ident, ItemFn, Path, Signature};

const ERR_APPLIED_TO_NON_FN: &str = "`with_test_telemetry` macro can only be used on functions";
const ERR_NON_TEST_FN: &str = "`with_test_telemetry` can wrap only `test` or `tokio::test`";

#[derive(FromMeta)]
struct Options {
    #[darling(default = "Options::default_crate_path")]
    crate_path: Path,

    #[darling(default)]
    rate_limit: Option<u32>,

    #[darling(multiple, rename = "redact_key")]
    redact_keys: Vec<String>,
}

impl Options {
    fn default_crate_path() -> Path {
        parse_quote!(::bedrock)
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

fn get_set_logging_settings_call(args: Args) -> Option<TokenStream2> {
    let crate_path = args.options.crate_path;
    let rate_limit_entry = args.options.rate_limit.map(|r| {
        quote!(
            rate_limit: #crate_path::telemetry::settings::LogRateLimitingSettings{
                enabled: true,
                max_events_per_second: #r,
            },
        )
    });

    let redact_keys_entry = if args.options.redact_keys.is_empty() {
        None
    } else {
        let redact_keys = StringVecTokenizer(args.options.redact_keys);
        Some(quote!(redact_keys: #redact_keys,))
    };

    rate_limit_entry.as_ref().or(redact_keys_entry.as_ref())?;

    let rate_limit_entry = rate_limit_entry.unwrap_or(quote!());
    let redact_keys_entry = redact_keys_entry.unwrap_or(quote!());

    Some(quote!(
        let logging_settings = #crate_path::telemetry::settings::LoggingSettings{
            #rate_limit_entry
            #redact_keys_entry
            ..Default::default()
        };

        __ctx.set_logging_settings(logging_settings);
    ))
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

    let (ctx_mut_modifier, set_logging_settings_call) = match get_set_logging_settings_call(args) {
        Some(s) => (quote!(mut), s),
        None => (quote!(), quote!()),
    };

    quote!(
        #(#attrs) *
        #[#test_attr]
        #vis #constness #unsafety #asyncness #abi fn #ident<#gen_params>() #return_type
        #where_clause
        {
            #inner_fn

            let #ctx_mut_modifier __ctx = #crate_path::telemetry::TelemetryContext::test();
            #set_logging_settings_call

            #wrapped_call
        }
    )
}

struct StringVecTokenizer(Vec<String>);

impl ToTokens for StringVecTokenizer {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let strings = self.0.iter().map(|s| s.as_str());
        tokens.extend(quote! {
            vec![ #( #strings.to_string() ),* ]
        });
    }
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

                let __ctx = ::bedrock::telemetry::TelemetryContext::test();

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

                let __ctx = ::bedrock::telemetry::TelemetryContext::test();
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
    fn expand_with_rate_limit() {
        let args = parse_attr! {
            #[with_test_telemetry(test, rate_limit = 10)]
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

                let mut __ctx = ::bedrock::telemetry::TelemetryContext::test();
                let logging_settings = ::bedrock::telemetry::settings::LoggingSettings{
                    rate_limit: ::bedrock::telemetry::settings::LogRateLimitingSettings {
                        enabled: true,
                        max_events_per_second: 10u32,
                    },
                    ..Default::default()
                };

                __ctx.set_logging_settings(logging_settings);
                let __scope = __ctx.scope();

                __some_test(__ctx);
            }
        };

        assert_eq!(actual, expected);
    }
    #[test]
    fn expand_with_redact_keys() {
        let args = parse_attr! {
            #[with_test_telemetry(test, redact_key = "foo", redact_key = "bar")]
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

                let mut __ctx = ::bedrock::telemetry::TelemetryContext::test();
                let logging_settings = ::bedrock::telemetry::settings::LoggingSettings{
                    redact_keys: vec!["foo".to_string(), "bar".to_string()],
                    ..Default::default()
                };

                __ctx.set_logging_settings(logging_settings);
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
