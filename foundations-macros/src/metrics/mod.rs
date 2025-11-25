use darling::util::Flag;
use darling::FromMeta;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, parse_quote, Attribute, ExprStruct, Ident, LitStr, Path, Token, Type,
    Visibility,
};

mod parsing;
mod validation;

#[derive(FromMeta)]
struct MacroArgs {
    unprefixed: Flag,
    #[darling(default = "Self::default_crate_path")]
    crate_path: Path,
}

impl Default for MacroArgs {
    fn default() -> Self {
        Self {
            unprefixed: Flag::default(),
            crate_path: Self::default_crate_path(),
        }
    }
}

impl MacroArgs {
    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }
}

struct Mod {
    attrs: Vec<Attribute>,
    vis: Visibility,
    mod_token: Token![mod],
    ident: Ident,
    fns: Vec<ItemFn>,
}

struct ItemFn {
    attrs: FnAttrs,
    vis: Visibility,
    fn_token: Token![fn],
    ident: Ident,
    args: Punctuated<FnArg, Token![,]>,
    arrow_token: Token![->],
    ty: Type,
}

#[derive(Default)]
struct FnAttrs {
    cfg: Vec<Attribute>,
    doc: String,
    ctor: Option<ExprStruct>,
    optional: bool,
    with_removal: bool,
}

struct FnArg {
    attrs: ArgAttrs,
    ident: Ident,
    colon_token: Token![:],
    ty: Type,
    mode: ArgMode,
}

#[derive(Default)]
struct ArgAttrs {
    serde: Option<Attribute>,
    serde_as: Option<Attribute>,
}

enum ArgMode {
    ByValue(Type),
    Clone(Type),
    Into(Type),
}

impl FnArg {
    fn to_struct_member(&self) -> proc_macro2::TokenStream {
        let Self {
            attrs: ArgAttrs { serde, serde_as },
            ident,
            colon_token,
            ..
        } = self;

        let ty = match &self.mode {
            ArgMode::ByValue(ty) => ty,
            ArgMode::Clone(ty) => ty,
            ArgMode::Into(ty) => ty,
        };

        quote! { #serde_as #serde #ident #colon_token #ty }
    }

    fn to_arg(&self) -> proc_macro2::TokenStream {
        let Self {
            ident,
            colon_token,
            ty,
            ..
        } = self;

        quote! { #ident #colon_token #ty }
    }

    fn to_initializer(&self) -> proc_macro2::TokenStream {
        let Self {
            ident, colon_token, ..
        } = self;

        match &self.mode {
            ArgMode::ByValue(_) => quote! { #ident },
            ArgMode::Clone(_) => quote! {
                #ident #colon_token ::std::clone::Clone::clone(#ident)
            },
            ArgMode::Into(_) => quote! {
                #ident #colon_token ::std::convert::Into::into(#ident)
            },
        }
    }
}

pub(crate) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as MacroArgs);
    let mod_ = parse_macro_input!(item as Mod);

    expand_from_parsed(args, mod_).into()
}

fn expand_from_parsed(args: MacroArgs, extern_: Mod) -> proc_macro2::TokenStream {
    let MacroArgs {
        unprefixed,
        crate_path: foundations,
    } = &args;
    let with_service_prefix =
        format_ident!("{}", !unprefixed.is_present(), span = unprefixed.span());

    let Mod {
        attrs: mod_attrs,
        vis: mod_vis,
        mod_token,
        ident: mod_name,
        fns,
    } = extern_;

    // This should be using `Span::def_site` but it is currently unstable.
    let metrics_struct = Ident::new(&format!("__{mod_name}_Metrics"), Span::call_site());

    let metric_fields = fns.iter().map(|fn_| metric_field(foundations, fn_));
    let label_set_structs = fns
        .iter()
        .filter_map(|fn_| label_set_struct(foundations, fn_));

    let registry_init = |var: &str, opt: bool| {
        let var = Ident::new(var, Span::call_site());
        let optional = format_ident!("{opt}");

        quote! {
            let #var = &mut *#foundations::telemetry::metrics::internal::Registries::get_subsystem(
                stringify!(#mod_name), #optional, #with_service_prefix
            );
        }
    };

    let init_registry = fns
        .iter()
        .any(|fn_| !fn_.attrs.optional)
        .then(|| registry_init("registry", false));

    let init_opt_registry = fns
        .iter()
        .any(|fn_| fn_.attrs.optional)
        .then(|| registry_init("opt_registry", true));

    let metric_inits = fns.iter().map(|fn_| metric_init(foundations, fn_));

    let metric_fns = fns
        .iter()
        .map(|fn_| metric_fn(foundations, &metrics_struct, fn_));

    quote! {
        #(#mod_attrs)* #mod_vis #mod_token #mod_name {
            use super::*;

            #[allow(non_camel_case_types)]
            struct #metrics_struct {
                #(#metric_fields,)*
            }

            #(#label_set_structs)*

            #[allow(non_upper_case_globals)]
            static #metrics_struct: ::std::sync::LazyLock<#metrics_struct> =
                ::std::sync::LazyLock::new(|| {
                    #init_registry
                    #init_opt_registry

                    #metrics_struct {
                        #(#metric_inits,)*
                    }
                });

            #(#metric_fns)*
        }
    }
}

/// Gets the type of the metric for its field in metric struct.
fn metric_field(foundations: &Path, fn_: &ItemFn) -> proc_macro2::TokenStream {
    let ItemFn {
        attrs: FnAttrs { cfg, ctor, .. },
        args,
        ty: metric_ty,
        ident: metric_name,
        ..
    } = fn_;

    let field_ty = if args.is_empty() {
        metric_ty.to_token_stream()
    } else if let Some(ExprStruct {
        path: ctor_path, ..
    }) = ctor
    {
        quote! {
            #foundations::reexports_for_macros::prometools::serde::Family<
                #metric_name,
                #metric_ty,
                #ctor_path,
            >
        }
    } else {
        quote! {
            #foundations::reexports_for_macros::prometools::serde::Family<
                #metric_name,
                #metric_ty,
            >
        }
    };

    quote! { #(#cfg)* #metric_name: #field_ty }
}

/// Returns the definition for the label set struct, if this metric uses labels.
fn label_set_struct(foundations: &Path, fn_: &ItemFn) -> Option<proc_macro2::TokenStream> {
    let ItemFn {
        attrs: FnAttrs { cfg, .. },
        args,
        ident: label_set_name,
        ..
    } = fn_;

    if args.is_empty() {
        return None;
    }

    let serde_with = quote! { #foundations::reexports_for_macros::serde_with };
    let serde_with_str = LitStr::new(&serde_with.to_string(), Span::call_site());

    let serde_as_attr = args
        .iter()
        .any(|arg| arg.attrs.serde_as.is_some())
        .then(|| quote! { #[#serde_with::serde_as(crate = #serde_with_str)] });

    let serde = quote! { #foundations::reexports_for_macros::serde };
    let serde_str = LitStr::new(&serde.to_string(), Span::call_site());

    let labels = args.iter().map(|arg| arg.to_struct_member());

    Some(quote! {
        #(#cfg)*
        #[allow(non_camel_case_types)]
        #serde_as_attr
        #[derive(
            ::std::clone::Clone,
            ::std::cmp::Eq,
            ::std::hash::Hash,
            ::std::cmp::PartialEq,
            #serde::Serialize,
        )]
        #[serde(crate = #serde_str)]
        struct #label_set_name {
            #(#labels,)*
        }
    })
}

fn metric_init(foundations: &Path, fn_: &ItemFn) -> proc_macro2::TokenStream {
    let ItemFn {
        attrs:
            FnAttrs {
                cfg,
                doc,
                optional,
                ctor,
                ..
            },
        ident: field_name,
        args,
        ..
    } = fn_;

    let reexports = quote! { #foundations::reexports_for_macros };
    let registry = Ident::new(
        if *optional {
            "opt_registry"
        } else {
            "registry"
        },
        Span::call_site(),
    );

    // Validate histogram buckets at compile time if this is a HistogramBuilder
    if let Some(ctor) = ctor {
        if let Some(path) = ctor.path.get_ident() {
            if path == "HistogramBuilder" {
                // Validate the histogram buckets
                if let Err(err) = validation::validate_histogram_buckets(ctor) {
                    return err.to_compile_error();
                }
            }
        }
    }

    let metric_init = match ctor {
        Some(ctor) if args.is_empty() => quote! {
            #reexports::prometheus_client::metrics::family::MetricConstructor::new_metric(&(#ctor))
        },
        Some(ctor) => quote! {
            #reexports::prometools::serde::Family::new_with_constructor(#ctor)
        },
        None => quote! { ::std::default::Default::default() },
    };

    quote! {
        #(#cfg)*
        #field_name: {
            let metric = #metric_init;

            #reexports::prometheus_client::registry::Registry::register(
                #registry,
                ::std::stringify!(#field_name),
                str::trim(#doc),
                ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
            );

            metric
        }
    }
}

fn metric_fn(foundations: &Path, metrics_struct: &Ident, fn_: &ItemFn) -> proc_macro2::TokenStream {
    let ItemFn {
        attrs:
            FnAttrs {
                cfg,
                doc,
                with_removal,
                ..
            },
        fn_token,
        vis: fn_vis,
        ident: metric_name,
        args,
        arrow_token,
        ty: metric_type,
    } = fn_;

    let fn_args: Vec<_> = args.iter().map(|arg| arg.to_arg()).collect();

    let (convert_args, access_metric) = if args.is_empty() {
        let accessor = quote! {
            ::std::clone::Clone::clone(&#metrics_struct.#metric_name)
        };
        (quote! {}, accessor)
    } else {
        let label_inits = args.iter().map(|arg| arg.to_initializer());
        let convert = quote! {
            let __args = #metric_name {
                #(#label_inits,)*
            };
        };

        let accessor = quote! {
            ::std::clone::Clone::clone(
                &#foundations::reexports_for_macros::prometools::serde::Family::get_or_create(
                    &#metrics_struct.#metric_name,
                    &__args,
                )
            )
        };
        (convert, accessor)
    };

    let removal_fns = if cfg!(foundations_unstable) && *with_removal {
        let remove_ident = format_ident!("{metric_name}_remove");
        let remove_doc = LitStr::new(
            &format!("Removes one label set from the `{metric_name}` family."),
            Span::call_site(),
        );

        let clear_ident = format_ident!("{metric_name}_clear");
        let clear_doc = LitStr::new(
            &format!("Removes all label sets from the `{metric_name}` family."),
            Span::call_site(),
        );

        quote! {
            #[doc = #remove_doc]
            #(#cfg)*
            #fn_vis #fn_token #remove_ident(#(#fn_args,)*) #arrow_token bool {
                #convert_args
                #foundations::reexports_for_macros::prometools::serde::Family::remove(
                    &#metrics_struct.#metric_name,
                    &__args,
                )
            }

            #[doc = #clear_doc]
            #(#cfg)*
            #fn_vis #fn_token #clear_ident() {
                #foundations::reexports_for_macros::prometools::serde::Family::clear(
                    &#metrics_struct.#metric_name,
                )
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[doc = #doc]
        #(#cfg)*
        #[must_use]
        #fn_vis #fn_token #metric_name(#(#fn_args,)*) #arrow_token #metric_type {
            #convert_args
            #access_metric
        }
        #removal_fns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use crate::metrics::validation;
    use syn::{parse_quote, ExprStruct};

    #[test]
    fn expand_empty() {
        let attr = parse_attr! {
            #[metrics]
        };

        let src = parse_quote! {
            #[attr]
            mod empty {}
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            #[attr]
            mod empty {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __empty_Metrics {}

                #[allow(non_upper_case_globals)]
                static __empty_Metrics: ::std::sync::LazyLock<__empty_Metrics> =
                    ::std::sync::LazyLock::new(|| { __empty_Metrics {} });
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_simple() {
        let attr = parse_attr! {
            #[metrics(crate_path = "tarmac")]
        };

        let src = parse_quote! {
            /// oxy metrics.
            pub mod oxy {
                /// Total number of connections
                pub fn connections_total() -> Counter;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            /// oxy metrics.
            pub mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    connections_total: Counter,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let registry = &mut *tarmac::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), false, true);

                        __oxy_Metrics {
                            connections_total: {
                                let metric = ::std::default::Default::default();

                                tarmac::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(connections_total),
                                    str::trim(" Total number of connections"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Total number of connections"]
                #[must_use]
                pub fn connections_total() -> Counter {
                    ::std::clone::Clone::clone(&__oxy_Metrics.connections_total)
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_simple_optional_only() {
        let attr = parse_attr! {
            #[metrics]
        };

        let src = parse_quote! {
            pub(crate) mod oxy {
                /// Total number of connections
                #[optional]
                pub(crate) fn connections_total() -> Counter;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            pub(crate) mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    connections_total: Counter,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let opt_registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), true, true);

                        __oxy_Metrics {
                            connections_total: {
                                let metric = ::std::default::Default::default();

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    opt_registry,
                                    ::std::stringify!(connections_total),
                                    str::trim(" Total number of connections"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Total number of connections"]
                #[must_use]
                pub(crate) fn connections_total() -> Counter {
                    ::std::clone::Clone::clone(&__oxy_Metrics.connections_total)
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_unprefixed_mixed() {
        let attr = parse_attr! {
            #[metrics(unprefixed)]
        };

        let src = parse_quote! {
            pub(crate) mod oxy {
                /// Total number of requests
                pub(crate) fn requests_total() -> Counter;

                /// Total number of connections
                #[optional]
                pub(crate) fn connections_total() -> Counter;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            pub(crate) mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    requests_total: Counter,
                    connections_total: Counter,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), false, false);
                        let opt_registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), true, false);

                        __oxy_Metrics {
                            requests_total: {
                                let metric = ::std::default::Default::default();

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(requests_total),
                                    str::trim(" Total number of requests"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                            connections_total: {
                                let metric = ::std::default::Default::default();

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    opt_registry,
                                    ::std::stringify!(connections_total),
                                    str::trim(" Total number of connections"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Total number of requests"]
                #[must_use]
                pub(crate) fn requests_total() -> Counter {
                    ::std::clone::Clone::clone(&__oxy_Metrics.requests_total)
                }

                #[doc = " Total number of connections"]
                #[must_use]
                pub(crate) fn connections_total() -> Counter {
                    ::std::clone::Clone::clone(&__oxy_Metrics.connections_total)
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_label_set() {
        let attr = parse_attr! {
            #[metrics]
        };

        let src = parse_quote! {
            pub mod oxy {
                /// Total number of connection errors
                pub fn connections_errors_total(
                    #[serde(rename = "endpoint_name")]
                    endpoint: &Arc<String>,
                    #[serde_as(as = "DisplayFromStr")]
                    kind: Kind,
                    message: &'static str,
                    error: impl Into<String>,
                ) -> Counter;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            pub mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    connections_errors_total:
                        ::foundations::reexports_for_macros::prometools::serde::Family<
                            connections_errors_total,
                            Counter,
                        >,
                }

                #[allow(non_camel_case_types)]
                #[::foundations::reexports_for_macros::serde_with::serde_as(crate = ":: foundations :: reexports_for_macros :: serde_with")]
                #[derive(
                    ::std::clone::Clone,
                    ::std::cmp::Eq,
                    ::std::hash::Hash,
                    ::std::cmp::PartialEq,
                    ::foundations::reexports_for_macros::serde::Serialize,
                )]
                #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
                struct connections_errors_total {
                    #[serde(rename = "endpoint_name")]
                    endpoint: Arc<String>,
                    #[serde_as(as = "DisplayFromStr")]
                    kind: Kind,
                    message: &'static str,
                    error: String,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), false, true);

                        __oxy_Metrics {
                            connections_errors_total: {
                                let metric = ::std::default::Default::default();

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(connections_errors_total),
                                    str::trim(" Total number of connection errors"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Total number of connection errors"]
                #[must_use]
                pub fn connections_errors_total(
                    endpoint: &Arc<String>,
                    kind: Kind,
                    message: &'static str,
                    error: impl Into<String>,
                ) -> Counter {
                    let __args = connections_errors_total {
                        endpoint: ::std::clone::Clone::clone(endpoint),
                        kind,
                        message,
                        error: ::std::convert::Into::into(error),
                    };
                    ::std::clone::Clone::clone(
                        &::foundations::reexports_for_macros::prometools::serde::Family::get_or_create(
                            &__oxy_Metrics.connections_errors_total,
                            &__args,
                        )
                    )
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_ctor() {
        let attr = parse_attr! {
            #[metrics]
        };

        let src = parse_quote! {
            pub mod oxy {
                /// Latency of connections
                #[ctor = HistogramBuilder { buckets: &[0.5, 1.] }]
                pub fn connections_latency() -> Histogram;

                /// Number of requests per connection
                #[ctor = HistogramBuilder { buckets: &[2., 3.] }]
                pub fn requests_per_connection(endpoint: String) -> Histogram;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            pub mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    connections_latency: Histogram,
                    requests_per_connection:
                        ::foundations::reexports_for_macros::prometools::serde::Family<
                            requests_per_connection,
                            Histogram,
                            HistogramBuilder,
                        >,
                }

                #[allow(non_camel_case_types)]
                #[derive(
                    ::std::clone::Clone,
                    ::std::cmp::Eq,
                    ::std::hash::Hash,
                    ::std::cmp::PartialEq,
                    ::foundations::reexports_for_macros::serde::Serialize,
                )]
                #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
                struct requests_per_connection {
                    endpoint: String,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), false, true);

                        __oxy_Metrics {
                            connections_latency: {
                                let metric = ::foundations::reexports_for_macros::prometheus_client::metrics::family::MetricConstructor::new_metric(
                                    &(HistogramBuilder { buckets: &[0.5, 1.] })
                                );

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(connections_latency),
                                    str::trim(" Latency of connections"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                            requests_per_connection: {
                                let metric = ::foundations::reexports_for_macros::prometools::serde::Family::new_with_constructor(
                                    HistogramBuilder { buckets: &[2., 3.] }
                                );

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(requests_per_connection),
                                    str::trim(" Number of requests per connection"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Latency of connections"]
                #[must_use]
                pub fn connections_latency() -> Histogram {
                    ::std::clone::Clone::clone(&__oxy_Metrics.connections_latency)
                }

                #[doc = " Number of requests per connection"]
                #[must_use]
                pub fn requests_per_connection(
                    endpoint: String,
                ) -> Histogram {
                    let __args = requests_per_connection { endpoint, };
                    ::std::clone::Clone::clone(
                        &::foundations::reexports_for_macros::prometools::serde::Family::get_or_create(
                            &__oxy_Metrics.requests_per_connection,
                            &__args,
                        )
                    )
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[cfg(foundations_unstable)]
    #[test]
    fn expand_with_removal() {
        let attr = parse_attr! {
            #[metrics]
        };

        let src = parse_quote! {
            pub(crate) mod oxy {
                /// Total number of requests
                #[with_removal]
                pub(crate) fn requests_total(status: u16) -> Counter;
            }
        };

        let actual = expand_from_parsed(attr, src).to_string();

        let expected = code_str! {
            pub(crate) mod oxy {
                use super::*;

                #[allow(non_camel_case_types)]
                struct __oxy_Metrics {
                    requests_total:
                        ::foundations::reexports_for_macros::prometools::serde::Family<
                            requests_total,
                            Counter,
                        >,
                }

                #[allow(non_camel_case_types)]
                #[derive(
                    ::std::clone::Clone,
                    ::std::cmp::Eq,
                    ::std::hash::Hash,
                    ::std::cmp::PartialEq,
                    ::foundations::reexports_for_macros::serde::Serialize,
                )]
                #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
                struct requests_total {
                    status: u16,
                }

                #[allow(non_upper_case_globals)]
                static __oxy_Metrics: ::std::sync::LazyLock<__oxy_Metrics> =
                    ::std::sync::LazyLock::new(|| {
                        let registry = &mut *::foundations::telemetry::metrics::internal::Registries::get_subsystem(stringify!(oxy), false, true);

                        __oxy_Metrics {
                            requests_total: {
                                let metric = ::std::default::Default::default();

                                ::foundations::reexports_for_macros::prometheus_client::registry::Registry::register(
                                    registry,
                                    ::std::stringify!(requests_total),
                                    str::trim(" Total number of requests"),
                                    ::std::boxed::Box::new(::std::clone::Clone::clone(&metric))
                                );

                                metric
                            },
                        }
                    });

                #[doc = " Total number of requests"]
                #[must_use]
                pub(crate) fn requests_total(status: u16,) -> Counter {
                    let __args = requests_total { status, };
                    ::std::clone::Clone::clone(
                        &::foundations::reexports_for_macros::prometools::serde::Family::get_or_create(
                            &__oxy_Metrics.requests_total,
                            &__args,
                        )
                    )
                }

                #[doc = "Removes one label set from the `requests_total` family."]
                pub(crate) fn requests_total_remove(status: u16,) -> bool {
                    let __args = requests_total { status, };
                    ::foundations::reexports_for_macros::prometools::serde::Family::remove(
                        &__oxy_Metrics.requests_total,
                        &__args,
                    )
                }

                #[doc = "Removes all label sets from the `requests_total` family."]
                pub(crate) fn requests_total_clear() {
                    ::foundations::reexports_for_macros::prometools::serde::Family::clear(
                        &__oxy_Metrics.requests_total,
                    )
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_valid_histogram_buckets() {
        // Valid strictly increasing buckets
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: &[0.1, 0.2, 0.3, 0.4] }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_ok());
    }

    #[test]
    fn test_empty_histogram_buckets() {
        // Empty buckets are valid
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: &[] }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_ok());
    }

    #[test]
    fn test_invalid_histogram_buckets_equal_values() {
        // Invalid buckets with equal values
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: &[0.1, 0.2, 0.2, 0.3] }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_err());
    }

    #[test]
    fn test_invalid_histogram_buckets_decreasing_values() {
        // Invalid buckets with decreasing values
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: &[0.3, 0.2, 0.1] }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_err());
    }

    #[test]
    fn test_skip_validation_for_variable_buckets() {
        // Variable buckets should skip validation
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: MY_BUCKETS }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_ok());
    }

    #[test]
    fn test_skip_validation_for_function_buckets() {
        // Function call buckets should skip validation
        let expr: ExprStruct = parse_quote! {
            HistogramBuilder { buckets: get_buckets() }
        };

        assert!(validation::validate_histogram_buckets(&expr).is_ok());
    }
}
