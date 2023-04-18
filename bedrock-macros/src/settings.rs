use crate::common::{error, Result};
use darling::FromMeta;
use proc_macro::TokenStream;
use quote::{quote, quote_spanned, TokenStreamExt};
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, parse_quote, Attribute, AttributeArgs, Field, Fields, Item, ItemEnum,
    ItemStruct, Lit, LitStr, Meta, MetaNameValue, NestedMeta, Path,
};

static ERR_NOT_STRUCT_OR_ENUM: &str = "Settings should be either structure or enum.";

static ERR_NON_UNIT_OR_NEW_TYPE_VARIANT: &str =
    "Settings enum variant should either be a unit variant (e.g. `Enum::Foo`) \
    or a new type variant (e.g. `Enum::Foo(Bar)`).";

static ERR_TUPLE_STRUCT: &str =
    "Settings with unnamed fields can only be new type structures (e.g. `struct Millimeters(u8)`).";

static ERR_CONDITIONAL_FIELD_OR_VARIANT: &str =
    "Settings shouldn't have conditionally compiled fields or enum variant (`#[cfg(...)]`) and \
    should be uniform on all platforms. Instead, error on startup if certain options \
    are not supported on the current platform.";

#[derive(FromMeta)]
struct Args {
    #[darling(default = "Args::default_impl_default")]
    impl_default: bool,
    #[darling(default = "Args::default_crate_path")]
    crate_path: Path,
}

impl Args {
    fn default_impl_default() -> bool {
        true
    }

    fn default_crate_path() -> Path {
        parse_quote!(::bedrock)
    }
}

impl Default for Args {
    fn default() -> Self {
        Args {
            impl_default: Args::default_impl_default(),
            crate_path: Args::default_crate_path(),
        }
    }
}

pub(super) fn expand(args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let attr_args = parse_macro_input!(args as AttributeArgs);

    let args = match Args::from_list(&attr_args) {
        Ok(args) => args,
        Err(e) => return e.write_errors().into(),
    };

    expand_from_parsed(args, item)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn expand_from_parsed(args: Args, mut item: Item) -> Result<proc_macro2::TokenStream> {
    match item {
        Item::Enum(ref mut item) => expand_enum(args, item),
        Item::Struct(ref mut item) if matches!(item.fields, Fields::Unnamed(_)) => {
            expand_unnamed_field_struct(args, item)
        }
        Item::Struct(ref mut item) => expand_struct(args, item),
        _ => error(&item, ERR_NOT_STRUCT_OR_ENUM),
    }
}

fn expand_enum(args: Args, item: &mut ItemEnum) -> Result<proc_macro2::TokenStream> {
    for variant in &item.variants {
        let is_struct = matches!(variant.fields, Fields::Named(_));

        if is_struct || is_tuple(&variant.fields) {
            return error(&variant, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
        }

        for attr in &variant.attrs {
            if attr.path.is_ident("cfg") {
                return error(&attr.path, ERR_CONDITIONAL_FIELD_OR_VARIANT);
            }
        }
    }

    if args.impl_default {
        item.attrs.push(parse_quote!(#[derive(Default)]));
    }

    add_default_attrs(&args, &mut item.attrs);

    item.attrs
        .push(parse_quote!(#[serde(rename_all = "snake_case")]));

    let ident = item.ident.clone();
    let crate_path = args.crate_path;

    Ok(quote! {
        #item

        impl #crate_path::settings::Settings for #ident { }
    })
}

fn expand_unnamed_field_struct(
    args: Args,
    item: &mut ItemStruct,
) -> Result<proc_macro2::TokenStream> {
    if is_tuple(&item.fields) {
        return error(&item, ERR_TUPLE_STRUCT);
    }

    if args.impl_default {
        item.attrs.push(parse_quote!(#[derive(Default)]));
    }

    add_default_attrs(&args, &mut item.attrs);

    let ident = item.ident.clone();
    let crate_path = args.crate_path;

    Ok(quote! {
        #item

        impl #crate_path::settings::Settings for #ident { }
    })
}

fn expand_struct(args: Args, item: &mut ItemStruct) -> Result<proc_macro2::TokenStream> {
    add_default_attrs(&args, &mut item.attrs);

    // Make every field optional.
    item.attrs.push(parse_quote!(#[serde(default)]));

    let impl_settings = impl_settings_trait(&args, item)?;

    let impl_default = if args.impl_default {
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

fn add_default_attrs(args: &Args, attrs: &mut Vec<Attribute>) {
    let crate_path = &args.crate_path;
    let serde_path = quote!(#crate_path::reexports_for_macros::serde).to_string();

    attrs.push(parse_quote!(#[derive(
        Clone,
        Debug,
        #crate_path::reexports_for_macros::serde::Serialize,
        #crate_path::reexports_for_macros::serde::Deserialize,
    )]));

    attrs.push(parse_quote!(#[serde(crate = #serde_path)]));
}

fn impl_settings_trait(args: &Args, item: &ItemStruct) -> Result<proc_macro2::TokenStream> {
    let ident = item.ident.clone();
    let crate_path = &args.crate_path;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
    let mut doc_comments_impl = quote! {};

    for field in &item.fields {
        if let Some(name) = &field.ident {
            let span = field.ty.span();
            let name_str = name.to_string();

            doc_comments_impl.append_all(quote_spanned! { span=>
                let mut key = parent_key.to_vec();

                key.push(#name_str.into());

                #crate_path::settings::Settings::add_docs(&self.#name, &key, docs);
            });

            let docs = extract_doc_comments(&field.attrs)?;

            if !docs.is_empty() {
                doc_comments_impl.append_all(quote! {
                    docs.insert(key, &[#(#docs,)*][..]);
                });
            }
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

fn extract_doc_comments(attrs: &[Attribute]) -> Result<Vec<LitStr>> {
    let mut comments = vec![];

    for attr in attrs {
        if attr.path.is_ident("cfg") {
            return error(&attr.path, ERR_CONDITIONAL_FIELD_OR_VARIANT);
        }

        if !attr.path.is_ident("doc") {
            continue;
        }

        if let Ok(Meta::NameValue(MetaNameValue {
            lit: Lit::Str(lit_str),
            ..
        })) = attr.parse_meta()
        {
            comments.push(lit_str);
        }
    }

    Ok(comments)
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
        let cfg_attrs = field.attrs.iter().filter(|attr| attr.path.is_ident("cfg"));

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
        if !attr.path.is_ident("serde") {
            continue;
        }

        let Ok(Meta::List(list)) = attr.parse_meta() else { continue };

        for meta in list.nested {
            let NestedMeta::Meta(Meta::NameValue(mnv)) = meta else { continue };

            if !mnv.path.is_ident("default") {
                continue;
            }

            let Lit::Str(val) = mnv.lit else { continue };

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
    use crate::common::test_utils::code_str;
    use syn::parse_quote;

    #[test]
    fn expand_structure() {
        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(Default::default(), src)
            .unwrap()
            .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                boolean: bool,
                #[doc = r" An integer value."]
                integer: i32,
            }

            impl ::bedrock::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::bedrock::settings::Settings::add_docs(&self.boolean, &key, docs);
                    docs.insert(key, &[r" A boolean value.",][..]);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::bedrock::settings::Settings::add_docs(&self.integer, &key, docs);
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
    fn expand_structure_with_crate_path() {
        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(
            Args {
                crate_path: parse_quote!(::custom::path),
                ..Default::default()
            },
            src,
        )
        .unwrap()
        .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::custom::path::reexports_for_macros::serde::Serialize,
                ::custom::path::reexports_for_macros::serde::Deserialize,
            )]
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
        let src = parse_quote! {
            struct TestStruct {
                /// A boolean value.
                boolean: bool,

                /// An integer value.
                integer: i32,
            }
        };

        let actual = expand_from_parsed(
            Args {
                impl_default: false,
                ..Default::default()
            },
            src,
        )
        .unwrap()
        .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            #[serde(default)]
            struct TestStruct {
                #[doc = r" A boolean value."]
                boolean: bool,
                #[doc = r" An integer value."]
                integer: i32,
            }

            impl ::bedrock::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::bedrock::settings::Settings::add_docs(&self.boolean, &key, docs);
                    docs.insert(key, &[r" A boolean value.",][..]);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::bedrock::settings::Settings::add_docs(&self.integer, &key, docs);
                    docs.insert(key, &[r" An integer value.",][..]);
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_newtype_struct() {
        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(Default::default(), src)
            .unwrap()
            .to_string();

        let expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            struct TestStruct(u64);

            impl ::bedrock::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_newtype_struct_no_impl_default() {
        let src = parse_quote! {
            struct TestStruct(u64);
        };

        let actual = expand_from_parsed(
            Args {
                impl_default: false,
                ..Default::default()
            },
            src,
        )
        .unwrap()
        .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            struct TestStruct(u64);

            impl ::bedrock::settings::Settings for TestStruct { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_enum() {
        let src = parse_quote! {
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }
        };

        let actual = expand_from_parsed(Default::default(), src)
            .unwrap()
            .to_string();

        let expected = code_str! {
            #[derive(Default)]
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                #[default]
                UnitVariant,
                NewTypeVariant(String)
            }

            impl ::bedrock::settings::Settings for TestEnum { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_enum_no_impl_default() {
        let src = parse_quote! {
            enum TestEnum {
                UnitVariant,
                NewTypeVariant(String)
            }
        };

        let actual = expand_from_parsed(
            Args {
                impl_default: false,
                ..Default::default()
            },
            src,
        )
        .unwrap()
        .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            #[serde(rename_all="snake_case")]
            enum TestEnum {
                UnitVariant,
                NewTypeVariant(String)
            }

            impl ::bedrock::settings::Settings for TestEnum { }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn expand_tuple_struct() {
        let src = parse_quote! {
            struct TestStruct(u64, String);
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_TUPLE_STRUCT);
    }

    #[test]
    fn expand_enum_with_struct_variant() {
        let src = parse_quote! {
            enum TestEnum {
                Variant {
                    field: u64
                }
            }
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
    }

    #[test]
    fn expand_enum_with_tuple_variant() {
        let src = parse_quote! {
            enum TestEnum {
               Tuple(u64, String)
            }
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_NON_UNIT_OR_NEW_TYPE_VARIANT);
    }

    #[test]
    fn expand_struct_with_conditional_field() {
        let src = parse_quote! {
            struct TestStruct {
               #[cfg(test)]
               field1: u64,
               field2: String
            }
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_CONDITIONAL_FIELD_OR_VARIANT);
    }

    #[test]
    fn expand_struct_with_conditional_variant() {
        let src = parse_quote! {
            enum TestEnum {
               #[cfg(test)]
               Variant1,
               Variant2
            }
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_CONDITIONAL_FIELD_OR_VARIANT);
    }

    #[test]
    fn expand_non_struct_or_enum() {
        let src = parse_quote! {
            mod foo {
                const BAR: &str = "baz";
            }
        };

        let err = expand_from_parsed(Default::default(), src)
            .unwrap_err()
            .to_string();

        assert_eq!(err, ERR_NOT_STRUCT_OR_ENUM);
    }

    #[test]
    fn expand_with_serde_defaults() {
        let src = parse_quote! {
            struct TestStruct {
                #[serde(default = "TestStruct::default_boolean")]
                boolean: bool,

                #[serde(default = "TestStruct::default_integer")]
                integer: i32,
            }
        };

        let actual = expand_from_parsed(Default::default(), src)
            .unwrap()
            .to_string();

        let expected = code_str! {
            #[derive(
                Clone,
                Debug,
                ::bedrock::reexports_for_macros::serde::Serialize,
                ::bedrock::reexports_for_macros::serde::Deserialize,
            )]
            #[serde(crate = ":: bedrock :: reexports_for_macros :: serde")]
            #[serde(default)]
            struct TestStruct {
                #[serde(default = "TestStruct::default_boolean")]
                boolean: bool,
                #[serde(default = "TestStruct::default_integer")]
                integer: i32,
            }

            impl ::bedrock::settings::Settings for TestStruct {
                fn add_docs(
                    &self,
                    parent_key: &[String],
                    docs: &mut ::std::collections::HashMap<Vec<String>, &'static [&'static str]>
                ) {
                    let mut key = parent_key.to_vec();
                    key.push("boolean".into());
                    ::bedrock::settings::Settings::add_docs(&self.boolean, &key, docs);
                    let mut key = parent_key.to_vec();
                    key.push("integer".into());
                    ::bedrock::settings::Settings::add_docs(&self.integer, &key, docs);
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
