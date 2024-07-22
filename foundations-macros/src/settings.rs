use crate::common::{error, parse_meta_list, Result};
use darling::ast::NestedMeta;
use darling::FromMeta;
use proc_macro::TokenStream;
use quote::{quote, quote_spanned, TokenStreamExt};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, parse_quote, Attribute, Expr, ExprLit, Field, Fields, Ident, Item, ItemEnum,
    ItemStruct, Lit, LitStr, Meta, MetaNameValue, Path,
};

const ERR_NOT_STRUCT_OR_ENUM: &str = "Settings should be either structure or enum.";

const ERR_NON_UNIT_OR_NEW_TYPE_VARIANT: &str =
    "Settings enum variant should either be a unit variant (e.g. `Enum::Foo`) \
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
}

impl Default for Options {
    fn default() -> Self {
        Options {
            impl_default: Options::default_impl_default(),
            impl_debug: Options::default_impl_debug(),
            crate_path: Options::default_crate_path(),
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

    let ident = item.ident.clone();
    let crate_path = options.crate_path;

    Ok(quote! {
        #item

        impl #crate_path::settings::Settings for #ident { }
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
        impl_serde_aware_default(item)
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
    let mut impl_for_field = quote! {};

    let cfg_attrs = field
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .collect::<Vec<_>>();

    impl_for_field.append_all(quote_spanned! { span=>
        let mut key = parent_key.to_vec();

        key.push(#name_str.into());

        #crate_path::settings::Settings::add_docs(&self.#name, &key, docs);
    });

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

fn impl_serde_aware_default(item: &ItemStruct) -> proc_macro2::TokenStream {
    let name = &item.ident;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    let initializers = item.fields.iter().map(|field| {
        let name = field
            .ident
            .as_ref()
            .expect("should not generate field docs for tuple struct");

        let span = field.ty.span();

        let cfg_attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg"));

        let function_path = get_field_default_fn(field)
            .unwrap_or_else(|| quote_spanned! { span=> Default::default });

        quote_spanned! { span=> #(#cfg_attrs)* #name: #function_path() }
    });

    quote! {
        impl #impl_generics Default for #name #ty_generics #where_clause {
            fn default() -> Self {
                Self { #(#initializers,)* }
            }
        }
    }
}

fn get_field_default_fn(field: &Field) -> Option<proc_macro2::TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }

        let Meta::List(list) = &attr.meta else {
            continue;
        };

        let Ok(nested_meta_list) = NestedMeta::parse_meta_list(list.tokens.clone()) else {
            continue;
        };

        for meta in nested_meta_list {
            let NestedMeta::Meta(Meta::NameValue(mnv)) = meta else {
                continue;
            };

            if !mnv.path.is_ident("default") {
                continue;
            }

            let Expr::Lit(ExprLit {
                lit: Lit::Str(val), ..
            }) = mnv.value
            else {
                continue;
            };

            match val.parse() {
                Ok(tokens) => return Some(tokens),
                Err(_) => continue,
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::{code_str, parse_attr};
    use syn::parse_quote;

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

        let expected = code_str! {
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

            impl ::foundations::settings::Settings for TestEnum { }
        };

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

        let expected = code_str! {
            #[derive(
                Clone,
                ::foundations::reexports_for_macros::serde::Serialize,
                ::foundations::reexports_for_macros::serde::Deserialize,
            )]
            #[derive(Debug)]
            #[serde(crate = ":: foundations :: reexports_for_macros :: serde")]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                UnitVariant,
                NewTypeVariant(String)
            }

            impl ::foundations::settings::Settings for TestEnum { }
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
