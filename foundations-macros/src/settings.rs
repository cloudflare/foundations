use crate::common::{Result, error, parse_meta_list};
use darling::FromMeta;
use darling::util::{Flag, Override};
use proc_macro::TokenStream;
use quote::{ToTokens, TokenStreamExt, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, ExprLit, Field, Fields, Ident, Item, ItemEnum, ItemStruct, Lit, LitStr, Meta,
    MetaNameValue, Path, Token, Type, Variant, parse_macro_input, parse_quote,
};

const ERR_NOT_STRUCT_OR_ENUM: &str = "Settings should be either structure or enum.";

const ERR_NON_UNIT_OR_NEW_TYPE_VARIANT: &str = "Settings enum variant should either be a unit variant (e.g. `Enum::Foo`) \
    or a new type variant (e.g. `Enum::Foo(Bar)`).";

const ERR_TUPLE_STRUCT: &str =
    "Settings with unnamed fields can only be new type structures (e.g. `struct Millimeters(u8)`).";

#[derive(FromMeta)]
struct Options {
    #[darling(default = "Options::default_impl_default")]
    impl_default: bool,
    #[darling(default = "Options::default_impl_debug")]
    impl_debug: bool,
    #[darling(default = "Options::default_crate_path")]
    crate_path: Path,
    #[darling(default = "Options::default_deny_unknown_fields")]
    deny_unknown_fields: bool,
}

impl Options {
    fn default_impl_default() -> bool {
        true
    }

    fn default_impl_debug() -> bool {
        true
    }

    fn default_crate_path() -> Path {
        parse_quote!(::foundations)
    }

    fn default_deny_unknown_fields() -> bool {
        cfg!(feature = "settings_deny_unknown_fields_by_default")
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            impl_default: Options::default_impl_default(),
            impl_debug: Options::default_impl_debug(),
            crate_path: Options::default_crate_path(),
            deny_unknown_fields: Options::default_deny_unknown_fields(),
        }
    }
}

impl Parse for Options {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let options = if input.is_empty() {
            Default::default()
        } else {
            let meta_list = parse_meta_list(&input)?;
            Self::from_list(&meta_list)?
        };

        Ok(options)
    }
}

#[derive(FromMeta)]
#[darling(allow_unknown_fields)]
struct SerdeArgs {
    flatten: Flag,
    default: Option<Override<Path>>,
}

impl SerdeArgs {
    fn parse_from_attrs(attrs: &[Attribute]) -> Result<Option<Self>> {
        let Some(attr) = attrs.iter().find(|a| a.path().is_ident("serde")) else {
            return Ok(None);
        };

        Ok(Some(Self::from_meta(&attr.meta)?))
    }
}

pub(crate) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let options = parse_macro_input!(args as Options);
    let item = parse_macro_input!(item as Item);

    expand_from_parsed(options, item)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn expand_from_parsed(options: Options, mut item: Item) -> Result<proc_macro2::TokenStream> {
    match item {
        Item::Enum(ref mut item) => expand_enum(options, item),
        Item::Struct(ref mut item) if matches!(item.fields, Fields::Unnamed(_)) => {
            expand_unnamed_field_struct(options, item)
        }
        Item::Struct(ref mut item) => expand_struct(options, item),
        _ => error(&item, ERR_NOT_STRUCT_OR_ENUM),
    }
}

fn expand_enum(options: Options, item: &mut ItemEnum) -> Result<proc_macro2::TokenStream> {
    for variant in &item.variants {
        let is_struct = matches!(variant.fields, Fields::Named(_));

        if is_struct || is_tuple(&variant.fields) {
            return error(&variant, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
        }
    }

    if options.impl_default {
        item.attrs.push(parse_quote!(#[derive(Default)]));
    }

    add_default_attrs(&options, &mut item.attrs);

    item.attrs
        .push(parse_quote!(#[serde(rename_all = "snake_case")]));

    let impl_settings = impl_settings_trait_for_enum(&options, item);

    Ok(quote! {
        #item

        #impl_settings
    })
}

fn expand_unnamed_field_struct(
    options: Options,
    item: &mut ItemStruct,
) -> Result<proc_macro2::TokenStream> {
    if is_tuple(&item.fields) {
        return error(&item, ERR_TUPLE_STRUCT);
    }

    if options.impl_default {
        item.attrs.push(parse_quote!(#[derive(Default)]));
    }

    add_default_attrs(&options, &mut item.attrs);

    let ident = item.ident.clone();
    let crate_path = options.crate_path;

    Ok(quote! {
        #item

        impl #crate_path::settings::Settings for #ident { }
    })
}

fn expand_struct(options: Options, item: &mut ItemStruct) -> Result<proc_macro2::TokenStream> {
    add_default_attrs(&options, &mut item.attrs);

    // Make every field optional.
    item.attrs.push(parse_quote!(#[serde(default)]));

    let impl_settings = impl_settings_trait(&options, item)?;

    let impl_default = if options.impl_default {
        impl_serde_aware_default(item)?
    } else {
        quote!()
    };

    Ok(quote! {
        #item

        #impl_settings
        #impl_default
    })
}

fn is_tuple(fields: &Fields) -> bool {
    matches!(fields, Fields::Unnamed(f) if f.unnamed.len() > 1)
}

fn add_default_attrs(options: &Options, attrs: &mut Vec<Attribute>) {
    let crate_path = &options.crate_path;
    let serde_path = quote!(#crate_path::reexports_for_macros::serde).to_string();

    attrs.push(parse_quote!(#[derive(
        Clone,
        #crate_path::reexports_for_macros::serde::Serialize,
        #crate_path::reexports_for_macros::serde::Deserialize,
    )]));

    if options.impl_debug {
        attrs.push(parse_quote!(#[derive(Debug)]));
    }

    attrs.push(parse_quote!(#[serde(crate = #serde_path)]));

    if options.deny_unknown_fields {
        attrs.push(parse_quote!(#[serde(deny_unknown_fields)]));
    }
}

fn impl_settings_trait(options: &Options, item: &ItemStruct) -> Result<proc_macro2::TokenStream> {
    let ident = item.ident.clone();
    let crate_path = &options.crate_path;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
    let mut doc_comments_impl = quote! {};

    for field in &item.fields {
        if let Some(name) = &field.ident {
            let impl_for_field = impl_settings_trait_for_field(options, field, name);

            doc_comments_impl.append_all(impl_for_field);
        }
    }

    Ok(quote! {
        impl #impl_generics #crate_path::settings::Settings for #ident #ty_generics #where_clause {
            fn add_docs(
                &self,
                parent_key: &[String],
                docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>)
            {
                #doc_comments_impl
            }
        }
    })
}

fn impl_settings_trait_for_field(
    options: &Options,
    field: &Field,
    name: &Ident,
) -> proc_macro2::TokenStream {
    let crate_path = &options.crate_path;
    let span = field.ty.span();
    let name_str = name.to_string();
    let docs = extract_doc_comments(&field.attrs);

    let cfg_attrs = field
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .collect::<Vec<_>>();

    // foundations#42: Flattened structs need to be forwarded directly, rather than
    // under their own key.
    if is_serde_flattened(&field.attrs) {
        return quote_spanned! { span=>
            #(#cfg_attrs)*
            {
                #crate_path::settings::Settings::add_docs(&self.#name, parent_key, docs);
            }
        };
    }

    let mut impl_for_field = quote_spanned! { span=>
        let mut key = parent_key.to_vec();
        key.push(#name_str.into());
    };

    // foundations#150: `[T; 0]` used to impl Settings for `T: !Default`, but this
    // is not possible anymore. We thus can't call `Settings::add_docs` for such
    // fields, but it was a noop anyway.
    // TODO(lblocher): Remove this compatibility hack with the next major release
    if !is_array_zst(&field.ty) {
        impl_for_field.append_all(quote_spanned! { span =>
            #crate_path::settings::Settings::add_docs(&self.#name, &key, docs);
        });
    }

    if !docs.is_empty() {
        impl_for_field.append_all(quote! {
            docs.insert(key, &[#(#docs,)*][..]);
        });
    }

    if !cfg_attrs.is_empty() {
        impl_for_field = quote! {
            #(#cfg_attrs)*
            {
                #impl_for_field
            }
        }
    }

    impl_for_field
}

fn impl_settings_trait_for_enum(options: &Options, item: &ItemEnum) -> proc_macro2::TokenStream {
    let ident = item.ident.clone();
    let crate_path = &options.crate_path;

    // Documenting a new type variant makes the type it wraps reachable through `add_docs`, so
    // that type has to implement `Settings`. That is a new requirement on existing code, so it
    // stays behind `--cfg foundations_unstable` until the next breaking release.
    //
    // Nothing under a variant can be documented otherwise, so keep the default no-op `add_docs`.
    if !cfg!(foundations_unstable) || !item.variants.iter().any(is_documented_variant) {
        return quote! {
            impl #crate_path::settings::Settings for #ident { }
        };
    }

    let mut doc_comments_impl = quote! {};

    for variant in &item.variants {
        doc_comments_impl.append_all(impl_settings_trait_for_variant(options, variant));
    }

    quote! {
        impl #crate_path::settings::Settings for #ident {
            fn add_docs(
                &self,
                parent_key: &[String],
                docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>)
            {
                match self {
                    #doc_comments_impl
                }
            }
        }
    }
}

/// Returns whether a variant gets a key of its own in the serialized settings.
///
/// A unit variant serializes as a bare value, so there is no line to document. A skipped
/// variant never reaches the config at all, and the type it wraps doesn't have to implement
/// [`Settings`].
fn is_documented_variant(variant: &Variant) -> bool {
    matches!(variant.fields, Fields::Unnamed(_)) && !is_serde_skipped(&variant.attrs)
}

fn impl_settings_trait_for_variant(
    options: &Options,
    variant: &Variant,
) -> proc_macro2::TokenStream {
    let ident = &variant.ident;
    let span = variant.fields.span();

    let cfg_attrs = variant
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .collect::<Vec<_>>();

    if !is_documented_variant(variant) {
        let pattern = match variant.fields {
            Fields::Unnamed(_) => quote_spanned! { span=> Self::#ident(..) },
            _ => quote_spanned! { span=> Self::#ident },
        };

        return quote! {
            #(#cfg_attrs)*
            #pattern => {}
        };
    }

    let crate_path = &options.crate_path;
    let key_str = serde_variant_name(variant);
    let docs = extract_doc_comments(&variant.attrs);

    let mut impl_for_variant = quote_spanned! { span=>
        let mut key = parent_key.to_vec();
        key.push(#key_str.into());
        #crate_path::settings::Settings::add_docs(value, &key, docs);
    };

    if !docs.is_empty() {
        impl_for_variant.append_all(quote! {
            docs.insert(key, &[#(#docs,)*][..]);
        });
    }

    quote_spanned! { span=>
        #(#cfg_attrs)*
        Self::#ident(value) => {
            #impl_for_variant
        }
    }
}

fn extract_doc_comments(attrs: &[Attribute]) -> Vec<LitStr> {
    let mut comments = vec![];

    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }

        if let Meta::NameValue(MetaNameValue {
            value:
                Expr::Lit(ExprLit {
                    lit: Lit::Str(lit_str),
                    ..
                }),
            ..
        }) = &attr.meta
        {
            comments.push(lit_str.clone());
        }
    }

    comments
}

/// Returns whether `ty` is exactly `[T; 0]` (for some T)
fn is_array_zst(ty: &Type) -> bool {
    let Type::Array(array) = ty else {
        return false;
    };

    let mut expr = &array.len;
    while let Expr::Cast(cast) = expr {
        expr = &cast.expr;
    }

    let Expr::Lit(ExprLit {
        lit: Lit::Int(len_lit),
        ..
    }) = expr
    else {
        return false;
    };
    len_lit
        .base10_parse::<usize>()
        .map(|len| len == 0)
        .unwrap_or(false)
}

/// Returns whether `attrs` contains `serde(flatten)`.
fn is_serde_flattened(attrs: &[Attribute]) -> bool {
    matches!(
        SerdeArgs::parse_from_attrs(attrs),
        Ok(Some(args)) if args.flatten.is_present(),
    )
}

/// Returns the arguments of every `serde` attribute in `attrs`, including the ones written as
/// `cfg_attr(<predicate>, serde(...))`, which reach an attribute macro unexpanded.
fn serde_args(attrs: &[Attribute]) -> Vec<Meta> {
    let mut args = Vec::new();

    for attr in attrs {
        let meta = if attr.path().is_ident("serde") {
            attr.meta.clone()
        } else if attr.path().is_ident("cfg_attr") {
            let Ok(nested) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                continue;
            };

            // The first argument is the `cfg` predicate, the rest are the attributes it guards.
            let Some(meta) = nested
                .into_iter()
                .skip(1)
                .find(|meta| meta.path().is_ident("serde"))
            else {
                continue;
            };

            meta
        } else {
            continue;
        };

        let Meta::List(list) = meta else {
            continue;
        };

        if let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
            args.extend(nested);
        }
    }

    args
}

/// Returns whether `attrs` contains `serde(skip)`.
fn is_serde_skipped(attrs: &[Attribute]) -> bool {
    serde_args(attrs)
        .iter()
        .any(|arg| arg.path().is_ident("skip"))
}

/// Returns the key an enum variant serializes under.
fn serde_variant_name(variant: &Variant) -> String {
    for arg in serde_args(&variant.attrs) {
        if let Meta::NameValue(MetaNameValue {
            path,
            value:
                Expr::Lit(ExprLit {
                    lit: Lit::Str(name),
                    ..
                }),
            ..
        }) = &arg
            && path.is_ident("rename")
        {
            return name.value();
        }
    }

    // `expand_enum` puts `serde(rename_all = "snake_case")` on every settings enum, so an
    // un-renamed variant serializes under the snake case of its identifier.
    let ident = variant.ident.to_string();
    let mut name = String::with_capacity(ident.len());

    for (index, character) in ident.char_indices() {
        if index > 0 && character.is_uppercase() {
            name.push('_');
        }

        name.push(character.to_ascii_lowercase());
    }

    name
}

fn impl_serde_aware_default(item: &ItemStruct) -> Result<proc_macro2::TokenStream> {
    let name = &item.ident;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    let initializers = item
        .fields
        .iter()
        .map(|field| {
            let name = field
                .ident
                .as_ref()
                .expect("should not generate field docs for tuple struct");

            let span = field.ty.span();

            let cfg_attrs = field
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("cfg"));

            let function_path = SerdeArgs::parse_from_attrs(&field.attrs)?
                .and_then(|a| Some(a.default?.explicit()?.into_token_stream()))
                .unwrap_or_else(|| quote_spanned! { span=> Default::default });

            Ok(quote_spanned! { span=> #(#cfg_attrs)* #name: #function_path() })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        impl #impl_generics Default for #name #ty_generics #where_clause {
            fn default() -> Self {
                Self { #(#initializers,)* }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use syn::parse_quote;

    /// The `Settings` impl expected for an enum with a new type variant.
    ///
    /// Documenting a new type variant is behind `--cfg foundations_unstable`, see
    /// `impl_settings_trait_for_enum`. Without it the enum keeps the default no-op `add_docs`.
    fn test_enum_settings_impl() -> String {
        if cfg!(foundations_unstable) {
            code_str! {
                impl ::foundations::settings::Settings for TestEnum {
                    fn add_docs(
                        &self,
                        parent_key: &[String],
                        docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>)
                    {
                        match self {
                            Self::UnitVariant => {}
                            Self::NewTypeVariant(value) => {
                                let mut key = parent_key.to_vec();
                                key.push("new_type_variant".into());
                                ::foundations::settings::Settings::add_docs(value, &key, docs);
                            }
                        }
                    }
                }
            }
        } else {
            code_str! {
                impl ::foundations::settings::Settings for TestEnum { }
            }
        }
    }

    #[test]
    fn expand_structure() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                boolean: bool,
                #[doc = r" An integer value."]
                integer: i32,
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::foundations::settings::Settings::add_docs(&self.boolean, &key, docs);
                    docs.insert(key, &[r" A boolean value.",][..]);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::foundations::settings::Settings::add_docs(&self.integer, &key, docs);
                    docs.insert(key, &[r" An integer value.",][..]);
                }
            }

            impl Default for TestStruct {
                fn default() -> Self {
                    Self {
                        boolean: Default::default(),
                        integer: Default::default(),
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_with_cfg_attrs() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                #[cfg(feature = "foobar")]
                boolean: bool,

                /// An integer value.
                #[cfg(test)]
                #[cfg(target_os = "linux")]
                integer: i32,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                #[cfg(feature = "foobar")]
                boolean: bool,
                #[doc = r" An integer value."]
                #[cfg(test)]
                #[cfg(target_os = "linux")]
                integer: i32,
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    #[cfg(feature = "foobar")]
                    {
                        let mut key = parent_key.to_vec();
                        key.push("boolean".into());
                        ::foundations::settings::Settings::add_docs(&self.boolean, &key, docs);
                        docs.insert(key, &[r" A boolean value.",][..]);
                    }
                    #[cfg(test)]
                    #[cfg(target_os = "linux")]
                    {
                        let mut key = parent_key.to_vec();
                        key.push("integer".into());
                        ::foundations::settings::Settings::add_docs(&self.integer, &key, docs);
                        docs.insert(key, &[r" An integer value.",][..]);
                    }
                }
            }

            impl Default for TestStruct {
                fn default() -> Self {
                    Self {
                        #[cfg(feature = "foobar")]
                        boolean: Default::default(),
                        #[cfg(test)]
                        #[cfg(target_os = "linux")]
                        integer: Default::default(),
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_with_crate_path() {
        let options = parse_attr! {
            #[settings(crate_path = "::custom::path")]
        };

        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::custom::path::reexports_for_macros::serde::Serialize,
                ::custom::path::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: custom :: path :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                boolean: bool,
                #[doc = r" An integer value."]
                integer: i32,
            }

            impl ::custom::path::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::custom::path::settings::Settings::add_docs(&self.boolean, &key, docs);
                    docs.insert(key, &[r" A boolean value.",][..]);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::custom::path::settings::Settings::add_docs(&self.integer, &key, docs);
                    docs.insert(key, &[r" An integer value.",][..]);
                }
            }

            impl Default for TestStruct {
                fn default() -> Self {
                    Self {
                        boolean: Default::default(),
                        integer: Default::default(),
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_no_impl_default() {
        let options = parse_attr! {
            #[settings(impl_default = false)]
        };

        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                boolean: bool,
                #[doc = r" An integer value."]
                integer: i32,
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::foundations::settings::Settings::add_docs(&self.boolean, &key, docs);
                    docs.insert(key, &[r" A boolean value.",][..]);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::foundations::settings::Settings::add_docs(&self.integer, &key, docs);
                    docs.insert(key, &[r" An integer value.",][..]);
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_empty_no_deny_unknown_fields() {
        let options = parse_attr! {
            #[settings(deny_unknown_fields = false)]
        };

        let src = parse_quote! {
            struct TestStruct {
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(default)]
            struct TestStruct {
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                }
            }

            impl Default for TestStruct {
                fn default() -> Self {
                    Self {
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_structure_with_flattened_member() {
        let options = parse_attr! {
            #[settings(impl_default = false)]
        };

        let src = parse_quote! {
            struct TestStruct {
                /// A flattened struct.
                #[serde(flatten)]
                embedded: OtherSettings,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A flattened struct."]
                #[serde(flatten)]
                embedded: OtherSettings,
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    {
                        ::foundations::settings::Settings::add_docs(&self.embedded, parent_key, docs);
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_newtype_struct() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            struct TestStruct(u64);

            impl ::foundations::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_no_impl_debug() {
        let options = parse_attr! {
            #[settings(impl_debug = false)]
        };

        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            struct TestStruct(u64);

            impl ::foundations::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_newtype_no_deny_unknown_fields() {
        let options = parse_attr! {
            #[settings(deny_unknown_fields = false)]
        };

        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            struct TestStruct(u64);

            impl ::foundations::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_newtype_struct_no_impl_default() {
        let options = parse_attr! {
            #[settings(impl_default = false)]
        };

        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            struct TestStruct(u64);

            impl ::foundations::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_enum() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let mut expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }
        };
        expected.push(' ');
        expected.push_str(&test_enum_settings_impl());

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_enum_no_impl_default() {
        let options = parse_attr! {
            #[settings(impl_default = false)]
        };

        let src = parse_quote! {
            enum TestEnum {
                UnitVariant,
                NewTypeVariant(String)
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let mut expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                UnitVariant,
                NewTypeVariant(String)
            }
        };
        expected.push(' ');
        expected.push_str(&test_enum_settings_impl());

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_enum_no_deny_unknown_fields() {
        let options = parse_attr! {
            #[settings(deny_unknown_fields = false)]
        };

        let src = parse_quote! {
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let mut expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }
        };
        expected.push(' ');
        expected.push_str(&test_enum_settings_impl());

        assert_eq!(actual, expected);
    }

    #[cfg(foundations_unstable)]
    #[test]
    fn expand_enum_with_variant_docs() {
        let options = parse_attr! {
            #[settings(impl_default = false)]
        };

        let src = parse_quote! {
            enum TestEnum {
                /// Nested settings.
                Nested(NestedStruct),
                /// Renamed variant.
                #[serde(rename = "RENAMED")]
                Renamed(NestedStruct),
                /// Not serialized.
                #[serde(skip)]
                Skipped(NotSettings),
                /// A unit variant.
                UnitVariant
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                #[doc = r" Nested settings."]
                Nested(NestedStruct),
                #[doc = r" Renamed variant."]
                #[serde(rename = "RENAMED")]
                Renamed(NestedStruct),
                #[doc = r" Not serialized."]
                #[serde(skip)]
                Skipped(NotSettings),
                #[doc = r" A unit variant."]
                UnitVariant
            }

            impl ::foundations::settings::Settings for TestEnum {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>)
                {
                    match self {
                        Self::Nested(value) => {
                            let mut key = parent_key.to_vec();
                            key.push("nested".into());
                            ::foundations::settings::Settings::add_docs(value, &key, docs);
                            docs.insert(key, &[r" Nested settings.",][..]);
                        }
                        Self::Renamed(value) => {
                            let mut key = parent_key.to_vec();
                            key.push("RENAMED".into());
                            ::foundations::settings::Settings::add_docs(value, &key, docs);
                            docs.insert(key, &[r" Renamed variant.",][..]);
                        }
                        Self::Skipped(..) => {}
                        Self::UnitVariant => {}
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_tuple_struct() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            struct TestStruct(u64, String);
        };

        let err = expand_from_parsed(options, src).unwrap_err().to_string();

        assert_eq!(err, ERR_TUPLE_STRUCT);
    }

    #[test]
    fn expand_enum_with_struct_variant() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            enum TestEnum {
                Variant {
                    field: u64
                }
            }
        };

        let err = expand_from_parsed(options, src).unwrap_err().to_string();

        assert_eq!(err, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
    }

    #[test]
    fn expand_enum_with_tuple_variant() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            enum TestEnum {
               Tuple(u64, String)
            }
        };

        let err = expand_from_parsed(options, src).unwrap_err().to_string();

        assert_eq!(err, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
    }

    #[test]
    fn expand_non_struct_or_enum() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            mod foo {
                const BAR: &str = "baz";
            }
        };

        let err = expand_from_parsed(options, src).unwrap_err().to_string();

        assert_eq!(err, ERR_NOT_STRUCT_OR_ENUM);
    }

    #[test]
    fn expand_with_serde_defaults() {
        let options = parse_attr! {
            #[settings]
        };

        let src = parse_quote! {
            struct TestStruct {
                #[serde(default = "TestStruct::default_boolean")]
                boolean: bool,

                #[serde(default = "TestStruct::default_integer")]
                integer: i32,
            }
        };

        let actual = expand_from_parsed(options, src).unwrap().to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(deny_unknown_fields)]
            #[serde(default)]
            struct TestStruct {
                #[serde(default = "TestStruct::default_boolean")]
                boolean: bool,
                #[serde(default = "TestStruct::default_integer")]
                integer: i32,
            }

            impl ::foundations::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::foundations::settings::Settings::add_docs(&self.boolean, &key, docs);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::foundations::settings::Settings::add_docs(&self.integer, &key, docs);
                }
            }

            impl Default for TestStruct {
                fn default() -> Self {
                    Self {
                        boolean: TestStruct::default_boolean(),
                        integer: TestStruct::default_integer(),
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }
}
