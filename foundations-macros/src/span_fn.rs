use crate::common::parse_optional_trailing_meta_list;
use crate::span_with_probe;
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned as _;
use syn::{Block, Expr, ExprCall, ItemFn, LitStr, Path, Signature, Stmt, parse_quote};

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

    #[darling(default = "Options::default_async_local")]
    async_local: bool,

    #[darling(default = "Options::default_generic")]
    generic: bool,

    #[darling(default = "Options::default_user")]
    user: bool,

    /// Add a per-span USDT probe fired at span end (`span_with_probe!`
    /// semantics).
    #[darling(default)]
    end_probe: bool,

    /// USDT provider for the probe (defaults to `$FOUNDATIONS_USDT_PROVIDER`
    /// or `"foundations"`); only valid together with `end_probe = true`.
    #[darling(default)]
    usdt_provider: Option<LitStr>,
}

impl Options {
    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }

    fn default_async_local() -> bool {
        false
    }

    fn default_generic() -> bool {
        cfg!(foundations_generic_telemetry_wrapper)
    }

    fn default_user() -> bool {
        false
    }
}

struct Args {
    span_name: SpanName,
    options: Options,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let span_name = input.parse::<SpanName>()?;
        let meta_list = parse_optional_trailing_meta_list(&input)?;
        let mut options = Options::from_list(&meta_list)?;

        if !options.end_probe
            && let Some(provider) = &options.usdt_provider
        {
            return Err(syn::Error::new(
                provider.span(),
                "usdt_provider requires end_probe = true",
            ));
        }

        if options.end_probe {
            let provider = options
                .usdt_provider
                .get_or_insert_with(span_with_probe::Options::default_usdt_provider);

            if !span_with_probe::is_valid_usdt_provider(&provider.value()) {
                return Err(syn::Error::new(
                    provider.span(),
                    "usdt_provider must be non-empty and must not contain `:`",
                ));
            }
        }

        // The probe name is derived from the span name at compile time.
        if options.end_probe
            && let SpanName::Const(path) = &span_name
        {
            return Err(syn::Error::new(
                path.span(),
                "end_probe spans require a string literal span name",
            ));
        }

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

    let body = match asyncness {
        Some(_) => wrap_with_span(&args, quote!(async move { #block })),
        None => try_async_trait_fn_rewrite(&args, &block).unwrap_or_else(|| {
            let span_expr = span_expr(&args);

            match end_probe_setup(&args) {
                Some(probe_setup) => quote!(
                    #[allow(unused_mut)]
                    let mut __span = #span_expr;
                    #probe_setup
                    #block
                ),
                None => quote!(
                    let __span = #span_expr;
                    #block
                ),
            }
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
        Stmt::Expr(Expr::Call(ExprCall { func, args, .. }), ..) => (func, args),
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

    let async_block = wrap_with_span(args, async_block.into_token_stream());

    // NOTE: OXY-1023 we do instrumentation inside additional future, so boxed
    // future can capture telemetry context on poll if it was instrumented.
    *body_stmts_token_streams.last_mut().unwrap() = quote!(
        Box::pin(async move { #async_block })
    );

    Some(quote!(
        #(#body_stmts_token_streams)*
    ))
}

fn wrap_with_span(args: &Args, block: TokenStream2) -> TokenStream2 {
    let apply_fn = if args.options.async_local {
        quote!(apply_local)
    } else if args.options.generic {
        quote!(apply_generic)
    } else {
        quote!(apply)
    };

    let span_expr = span_expr(args);

    match end_probe_setup(args) {
        Some(probe_setup) => quote!(
            {
                #[allow(unused_mut)]
                let mut __span = #span_expr;
                #probe_setup
                __span
                    .into_context()
                    .#apply_fn(#block)
                    .await
            }
        ),
        None => quote!(
            #span_expr
                .into_context()
                .#apply_fn(#block)
                .await
        ),
    }
}

/// The span-construction expression: a plain `span`/`dual_span` call.
fn span_expr(args: &Args) -> TokenStream2 {
    let span_name = args.span_name.as_tokens();
    let crate_path = &args.options.crate_path;
    let span_ctor = span_ctor(&args.options);

    quote!(#crate_path::telemetry::tracing::#span_ctor(#span_name))
}

/// When `end_probe` is enabled, the linux/x86_64-only block that sets up the
/// USDT span-end probe on the just-created `__span`.
fn end_probe_setup(args: &Args) -> Option<TokenStream2> {
    if !args.options.end_probe {
        return None;
    }

    let SpanName::Str(span_name) = &args.span_name else {
        unreachable!("end_probe spans require a string literal span name");
    };

    let usdt_provider = args
        .options
        .usdt_provider
        .as_ref()
        .expect("provider defaulted and validated during parse");

    let probe_setup = span_with_probe::probe_setup(span_name, usdt_provider);
    let track_env = span_with_probe::track_provider_env();

    Some(quote!(
        #track_env

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            #probe_setup
        }
    ))
}

/// The span constructor to call: `dual_span` when `user = true` (internal + parallel user span),
/// otherwise `span`.
fn span_ctor(options: &Options) -> TokenStream2 {
    if options.user {
        quote!(dual_span)
    } else {
        quote!(span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use syn::parse_quote;

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
                let __span = ::foundations::telemetry::tracing::span("sync_span");
                {
                    do_something_else();

                    Ok("foo".into())
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_sync_fn_user() {
        let args = parse_attr! {
            #[span_fn("sync_span", user = true)]
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
                let __span = ::foundations::telemetry::tracing::dual_span("sync_span");
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
                let __span = ::foundations::telemetry::tracing::span(some::module::SYNC_SPAN);
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
                ::foundations::telemetry::tracing::span("async_span")
                    .into_context()
                    .apply(async move {{
                        do_something_else().await;

                        Ok("foo".into())
                    }})
                    .await
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_fn_user() {
        let args = parse_attr! {
            #[span_fn("async_span", user = true)]
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
                ::foundations::telemetry::tracing::dual_span("async_span")
                    .into_context()
                    .apply(async move {{
                        do_something_else().await;

                        Ok("foo".into())
                    }})
                    .await
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_fn_local() {
        let args = parse_attr! {
            #[span_fn("async_span", async_local = true)]
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
                ::foundations::telemetry::tracing::span("async_span")
                    .into_context()
                    .apply_local(async move {{
                        do_something_else().await;

                        Ok("foo".into())
                    }})
                    .await
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_fn_generic() {
        let args = parse_attr! {
            #[span_fn("async_span", generic = true)]
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
                ::foundations::telemetry::tracing::span("async_span")
                    .into_context()
                    .apply_generic(async move {{
                        do_something_else().await;

                        Ok("foo".into())
                    }})
                    .await
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
                    ::foundations::telemetry::tracing::span("async_trait_span")
                        .into_context()
                        .apply(async move {
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
                        .await
                })
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_trait_fn_local() {
        let args = parse_attr! {
            #[span_fn("async_trait_span", async_local = true)]
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
                    ::foundations::telemetry::tracing::span("async_trait_span")
                        .into_context()
                        .apply_local(async move {
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
                        .await
                })
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_async_trait_fn_generic() {
        let args = parse_attr! {
            #[span_fn("async_trait_span", generic = true)]
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
                    ::foundations::telemetry::tracing::span("async_trait_span")
                        .into_context()
                        .apply_generic(async move {
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
                        .await
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

    #[test]
    fn expand_sync_fn_probe() {
        let args = parse_attr! {
            #[span_fn("sync_span", end_probe = true)]
        };

        let item_fn = parse_quote! {
            fn do_sync() -> io::Result<String> {
                do_something_else();

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        // Plain span construction, with the probe armed on the scope after.
        assert!(actual.contains(
            "let mut __span = :: foundations :: telemetry :: tracing :: span (\"sync_span\") ;"
        ));
        assert!(actual.contains("if enabled { __span . __arm_probe (span_end_probe) ; }"));
        assert!(actual.contains(".asciz \\\"span_end__sync_span\\\""));
        assert!(actual.contains(".asciz \\\"foundations\\\""));
        // Probe arming is linux/x86_64-only.
        assert!(actual.contains("cfg (all (target_os = \"linux\" , target_arch = \"x86_64\"))"));
    }

    #[test]
    fn expand_async_fn_probe() {
        let args = parse_attr! {
            #[span_fn("async_span", end_probe = true)]
        };

        let item_fn = parse_quote! {
            async fn do_async() -> io::Result<String> {
                do_something_else().await;

                Ok("foo".into())
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        assert!(actual.contains(
            "let mut __span = :: foundations :: telemetry :: tracing :: span (\"async_span\") ;"
        ));
        assert!(actual.contains("if enabled { __span . __arm_probe (span_end_probe) ; }"));
        assert!(actual.contains("__span . into_context () . apply (async move"));
    }

    #[test]
    fn expand_fn_probe_user() {
        let args = parse_attr! {
            #[span_fn("user_span", end_probe = true, user = true)]
        };

        let item_fn = parse_quote! {
            fn do_sync() {
                do_something_else();
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        assert!(actual.contains(
            "let mut __span = :: foundations :: telemetry :: tracing :: dual_span (\"user_span\") ;"
        ));
        assert!(actual.contains("if enabled { __span . __arm_probe (span_end_probe) ; }"));
    }

    #[test]
    fn expand_fn_probe_with_crate_path() {
        let args = parse_attr! {
            #[span_fn("sync_span", end_probe = true, crate_path = "::foo::bar")]
        };

        let item_fn = parse_quote! {
            fn do_sync() {
                do_something_else();
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        assert!(actual.contains(
            "let mut __span = :: foo :: bar :: telemetry :: tracing :: span (\"sync_span\") ;"
        ));
        assert!(actual.contains("if enabled { __span . __arm_probe (span_end_probe) ; }"));
    }

    #[test]
    fn expand_fn_probe_with_usdt_provider() {
        let args = parse_attr! {
            #[span_fn("some::span", end_probe = true, usdt_provider = "myapp")]
        };

        let item_fn = parse_quote! {
            fn do_sync() {
                do_something_else();
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        // The asm template is a nested string literal, so its quotes are
        // escaped in the token stream's string representation.
        assert!(actual.contains(".asciz \\\"myapp\\\""));
        assert!(actual.contains(".asciz \\\"span_end__some__span\\\""));
    }

    #[test]
    fn expand_fn_probe_with_env_usdt_provider() {
        unsafe { std::env::set_var(span_with_probe::USDT_PROVIDER_ENV_VAR, "envapp") };

        let args = parse_attr! {
            #[span_fn("some::span", end_probe = true)]
        };

        let item_fn = parse_quote! {
            fn do_sync() {
                do_something_else();
            }
        };

        let actual = expand_from_parsed(args, item_fn).to_string();

        assert!(actual.contains(".asciz \\\"envapp\\\""));

        unsafe { std::env::remove_var(span_with_probe::USDT_PROVIDER_ENV_VAR) };
    }

    #[test]
    fn rejects_usdt_provider_without_probe() {
        let tokens = quote! { "some::span", usdt_provider = "myapp" };
        let err = match syn::parse2::<Args>(tokens) {
            Ok(_) => panic!("usdt_provider without probe unexpectedly accepted"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("requires end_probe = true"),
            "{err}"
        );
    }

    #[test]
    fn rejects_invalid_usdt_provider() {
        for provider in ["foo:bar", ""] {
            let tokens = quote! { "some::span", end_probe = true, usdt_provider = #provider };
            let err = match syn::parse2::<Args>(tokens) {
                Ok(_) => panic!("provider {provider:?} unexpectedly accepted"),
                Err(err) => err,
            };

            assert!(
                err.to_string().contains("usdt_provider"),
                "provider {provider:?}: {err}"
            );
        }
    }

    #[test]
    fn rejects_probe_with_const_span_name() {
        let tokens = quote! { some::module::SPAN, end_probe = true };
        let err = match syn::parse2::<Args>(tokens) {
            Ok(_) => panic!("const span name with probe unexpectedly accepted"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("string literal span name"),
            "{err}"
        );
    }
}
