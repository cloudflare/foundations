use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    parse_quote, Block, Expr, ExprCall, ItemFn, LitStr, NestedMeta, Path, Signature, Stmt, Token,
};

const ERR_APPLIED_TO_NON_FN: &str = "`span_fn` macro can only be used on functions";

#[derive(Debug)]
enum SpanName {
    Str(LitStr),
    Const(Path),
}

impl SpanName {
    fn as_tokens(&self) -> impl ToTokens {
        match self {
            SpanName::Str(lit) => quote!(#lit),
            SpanName::Const(path) => quote!(#path),
        }
    }
}

impl Parse for SpanName {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<LitStr>().map(SpanName::Str).or_else(|e1| {
            input
                .parse::<Path>()
                .map(SpanName::Const)
                .map_err(|mut e2| {
                    e2.combine(e1);
                    e2
                })
        })
    }
}

#[derive(FromMeta)]
struct Options {
    #[darling(default = "Options::default_crate_path")]
    crate_path: Path,
}

impl Options {
    fn default_crate_path() -> Path {
        parse_quote!(::bedrock)
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            crate_path: Options::default_crate_path(),
        }
    }
}

struct Args {
    span_name: SpanName,
    options: Options,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let span_name = input.parse::<SpanName>()?;

        let options = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;

            let meta_list = Punctuated::<NestedMeta, Token![,]>::parse_terminated(input)?
                .into_iter()
                .collect::<Vec<_>>();

            Options::from_list(&meta_list)?
        } else {
            Default::default()
        };

        Ok(Self { span_name, options })
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

    let span_name = args.span_name.as_tokens();
    let crate_path = &args.options.crate_path;

    let body = match asyncness {
        Some(_) => quote!(
            #crate_path::telemetry::TelemetryContext::current().apply_with_tracing_span(
                #span_name,
                async move { #block }
            ).await
        ),
        None => try_async_trait_fn_rewrite(&args, &block).unwrap_or_else(|| {
            quote!(
                let __span = #crate_path::telemetry::tracing::span(#span_name);
                #block
            )
        }),
    };

    quote!(
        #(#attrs) *
        #vis #constness #unsafety #asyncness #abi fn #ident<#gen_params>(#params) #return_type
        #where_clause
        {
            #body
        }
    )
}

fn try_async_trait_fn_rewrite(args: &Args, body: &Block) -> Option<TokenStream2> {
    let (last_expr_fn_call, last_expr_fn_call_args) = match body.stmts.last()? {
        Stmt::Expr(Expr::Call(ExprCall { func, args, .. })) => (func, args),
        _ => return None,
    };

    let fn_path_segments = match &**last_expr_fn_call {
        Expr::Path(path) => &path.path.segments,
        _ => return None,
    };

    let is_box_pin_call = fn_path_segments.len() == 2
        && fn_path_segments[0].ident == "Box"
        && fn_path_segments[1].ident == "pin";

    let is_async_block_arg =
        last_expr_fn_call_args.len() == 1 && matches!(last_expr_fn_call_args[0], Expr::Async(_));

    if !(is_box_pin_call && is_async_block_arg) {
        return None;
    }

    let async_block = &last_expr_fn_call_args[0];

    let mut body_stmts_token_streams: Vec<_> = body
        .stmts
        .iter()
        .map(|stmt| stmt.to_token_stream())
        .collect();

    let span_name = args.span_name.as_tokens();
    let crate_path = &args.options.crate_path;

    // NOTE: OXY-1023 we do instrumentation inside additional future, so boxed
    // future can capture telemetry context on poll if it was instrumented.
    *body_stmts_token_streams.last_mut().unwrap() = quote!(
        Box::pin(async move {
            #crate_path::telemetry::TelemetryContext::current().apply_with_tracing_span(
                #span_name,
                #async_block
            ).await
        })
    );

    Some(quote!(
        #(#body_stmts_token_streams)*
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::code_str;
    use syn::{parse_quote, Attribute};

    macro_rules! parse_attr {
        ( $($t:tt)* ) => {{
            let attr: Attribute = parse_quote! { $($t)* };

            attr.parse_args::<Args>().unwrap()
        }};
    }

    #[test]
    fn expand_sync_fn() {
        let args = parse_attr! {
            #[span_fn("sync_span")]
        };

        let item_fn = parse_quote! {
            fn do_sync() -> io::Result<String> {
                do_something_else();

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            fn do_sync<>() -> io::Result<String> {
                let __span = ::bedrock::telemetry::tracing::span("sync_span");
                {
                    do_something_else();

                    Ok("foo".into())
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_sync_fn_const_span_name() {
        let args = parse_attr! {
            #[span_fn(some::module::SYNC_SPAN)]
        };

        let item_fn = parse_quote! {
            fn do_sync() -> io::Result<String> {
                do_something_else();

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            fn do_sync<>() -> io::Result<String> {
                let __span = ::bedrock::telemetry::tracing::span(some::module::SYNC_SPAN);
                {
                    do_something_else();

                    Ok("foo".into())
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_fn() {
        let args = parse_attr! {
            #[span_fn("async_span")]
        };

        let item_fn = parse_quote! {
            async fn do_async() -> io::Result<String> {
                do_something_else().await;

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            async fn do_async<>() -> io::Result<String> {
                ::bedrock::telemetry::TelemetryContext::current().apply_with_tracing_span(
                    "async_span",
                    async move {{
                        do_something_else().await;

                        Ok("foo".into())
                    }}
                ).await
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_trait_fn() {
        let args = parse_attr! {
            #[span_fn("async_trait_span")]
        };

        let item_fn = parse_quote! {
            fn test<'life0, 'async_trait>(
                &'life0 self,
            ) -> ::core::pin::Pin<
                Box<dyn ::core::future::Future<Output = String> + ::core::marker::Send + 'async_trait>
            >
            where
                'life0: 'async_trait,
                Self: 'async_trait,
            {
                Box::pin(async move {
                    if let ::core::option::Option::Some(__ret) = ::core::option::Option::None::<String> {
                        return __ret;
                    }
                    let __self = self;
                    let __ret: String = {
                        __self.do_something_else().await;
                        "foo".into()
                    };
                    #[allow(unreachable_code)]
                    __ret
                })
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            fn test<'life0, 'async_trait>(
                &'life0 self,
            ) -> ::core::pin::Pin<
                Box<dyn ::core::future::Future<Output = String> + ::core::marker::Send + 'async_trait>
            >
            where
                'life0: 'async_trait,
                Self: 'async_trait,
            {
                Box::pin(async move {
                    ::bedrock::telemetry::TelemetryContext::current().apply_with_tracing_span(
                        "async_trait_span",
                        async move {
                            if let ::core::option::Option::Some(__ret) = ::core::option::Option::None::<String> {
                                return __ret;
                            }
                            let __self = self;
                            let __ret: String = {
                                __self.do_something_else().await;
                                "foo".into()
                            };
                            #[allow(unreachable_code)]
                            __ret
                        }).await
                })
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_with_crate_path() {
        let args = parse_attr! {
            #[span_fn("sync_span", crate_path = "::foo::bar")]
        };

        let item_fn = parse_quote! {
            fn do_sync() -> io::Result<String> {
                do_something_else();

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        let expected = code_str! {
            fn do_sync<>() -> io::Result<String> {
                let __span = ::foo::bar::telemetry::tracing::span("sync_span");
                {
                    do_something_else();

                    Ok("foo".into())
                }
            }
        };

        assert_eq!(actual, expected);
    }
}
