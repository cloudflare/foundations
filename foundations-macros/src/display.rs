use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields, LitStr};

#[derive(Clone, Copy)]
pub(crate) enum DisplayOptions {
    None,
    ImplError,
}

pub(crate) fn expand(
    input: proc_macro::TokenStream,
    options: DisplayOptions,
) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    expand_from_parsed(input, options)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_from_parsed(input: DeriveInput, options: DisplayOptions) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let formatter = syn::Ident::new("__foundations_formatter", Span::mixed_site());

    let body = match &input.data {
        Data::Struct(data) => {
            let (pattern, output) = format_fields(&input.attrs, &data.fields)?;
            quote! {
                let Self #pattern = self;
                #output
            }
        }
        Data::Enum(data) => {
            let arms = expand_enum(&input.attrs, data)?;
            quote! { match self { #(#arms),* } }
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "Display cannot be derived for unions",
            ));
        }
    };

    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    let display_impl = quote! {
        #[automatically_derived]
        impl #impl_generics ::std::fmt::Display for #name #type_generics #where_clause {
            fn fmt(&self, #formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                #body
            }
        }
    };

    if matches!(options, DisplayOptions::ImplError) {
        Ok(quote! {
            #display_impl
            #[automatically_derived]
            impl #impl_generics ::std::error::Error for #name #type_generics #where_clause {}
        })
    } else {
        Ok(display_impl)
    }
}

fn expand_enum(attrs: &[Attribute], data: &syn::DataEnum) -> syn::Result<Vec<TokenStream>> {
    if let Some(attribute) = find_display_attr(attrs) {
        return Err(syn::Error::new_spanned(
            attribute,
            "put #[display(\"...\")] on each variant",
        ));
    }

    data.variants
        .iter()
        .map(|variant| {
            let variant_name = &variant.ident;
            let (pattern, output) = format_fields(&variant.attrs, &variant.fields)?;
            Ok(quote! { Self::#variant_name #pattern => { #output } })
        })
        .collect::<syn::Result<Vec<_>>>()
}

fn format_fields(
    attributes: &[Attribute],
    fields: &Fields,
) -> syn::Result<(TokenStream, TokenStream)> {
    let formatter = syn::Ident::new("__foundations_formatter", Span::mixed_site());
    let mut display_attributes = attributes.iter().filter(is_display_attr);

    let attribute = display_attributes.next().ok_or_else(|| {
        syn::Error::new_spanned(
            fields,
            "expected #[display(\"...\")] with one format string",
        )
    })?;

    let message = attribute.parse_args::<LitStr>()?;

    if let Some(duplicate) = display_attributes.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "duplicate #[display] attribute",
        ));
    }

    for field in fields {
        if let Some(attribute) = find_display_attr(&field.attrs) {
            return Err(syn::Error::new_spanned(
                attribute,
                "field display attributes are not supported",
            ));
        }
    }

    let (pattern, bindings, message) = match fields {
        Fields::Named(fields) => {
            let names = fields
                .named
                .iter()
                .map(|field| field.ident.as_ref().unwrap())
                .collect::<Vec<_>>();
            (
                quote! { { #(#names),* } },
                quote! { #(let _ = #names;)* },
                message,
            )
        }
        Fields::Unnamed(fields) => {
            let names = (0..fields.unnamed.len())
                .map(|index| format_ident!("__foundations_field_{index}"))
                .collect::<Vec<_>>();
            let message = rewrite_format_message_for_tuples(&message, names.len())?;
            (
                quote! { (#(#names),*) },
                quote! { #(let _ = #names;)* },
                message,
            )
        }
        Fields::Unit => (quote! {}, quote! {}, message),
    };

    Ok((
        pattern,
        quote! {
            #bindings
            ::std::write!(#formatter, #message)
        },
    ))
}

fn is_display_attr(attr: &&Attribute) -> bool {
    attr.path().is_ident("display")
}

fn find_display_attr(attrs: &[Attribute]) -> Option<&Attribute> {
    attrs.iter().find(is_display_attr)
}

fn rewrite_format_message_for_tuples(message: &LitStr, field_count: usize) -> syn::Result<LitStr> {
    #[derive(Clone, PartialEq, Eq)]
    enum State {
        Text,
        OpenBrace,
        Argument(String),
        FormatSpec,
    }
    #[derive(Clone, Copy)]
    enum ArgType {
        Implicit,
        Explicit,
    }

    let text = message.value();
    let mut output = String::with_capacity(text.len());
    let mut state = State::Text;
    let mut implicit_index = 0;
    let mut explicit_indexing = None;

    let mut resolve_argument = |argument: &str| -> syn::Result<usize> {
        let arg_type = match argument.is_empty() {
            true => ArgType::Implicit,
            false => ArgType::Explicit,
        };

        match (explicit_indexing, arg_type) {
            (None, _) => explicit_indexing = Some(arg_type),
            (Some(ArgType::Implicit), ArgType::Explicit)
            | (Some(ArgType::Explicit), ArgType::Implicit) => {
                return Err(syn::Error::new_spanned(
                    message,
                    "cannot mix implicit (`{}`) and explicit (`{0}`) placeholders in the same format string",
                ));
            }
            _ => {}
        }

        let index = match arg_type {
            ArgType::Explicit => argument.parse::<usize>().map_err(|_| {
                syn::Error::new_spanned(
                    message,
                    "tuple placeholders must use positional field indices",
                )
            })?,
            ArgType::Implicit => {
                let index = implicit_index;
                implicit_index += 1;
                index
            }
        };

        if index >= field_count {
            return Err(syn::Error::new_spanned(
                message,
                "tuple field index is out of range",
            ));
        }

        Ok(index)
    };

    for character in text.chars() {
        match state {
            State::Text => {
                output.push(character);
                if character == '{' {
                    state = State::OpenBrace;
                }
            }
            State::OpenBrace => {
                if character == '{' {
                    // Escaped opening brace: keep both braces.
                    output.push(character);
                    state = State::Text;
                } else if character == '}' {
                    // Empty placeholder `{}`.
                    let index = resolve_argument("")?;
                    output.push_str(&format!("__foundations_field_{index}"));
                    output.push('}');
                    state = State::Text;
                } else if character == ':' {
                    // Empty argument with a format spec `{:<spec>}`.
                    let index = resolve_argument("")?;
                    output.push_str(&format!("__foundations_field_{index}"));
                    output.push(':');
                    state = State::FormatSpec;
                } else {
                    state = State::Argument(character.to_string());
                }
            }
            State::Argument(ref mut argument) => {
                if character == '}' || character == ':' {
                    let index = resolve_argument(argument)?;
                    output.push_str(&format!("__foundations_field_{index}"));
                    output.push(character);
                    state = if character == ':' {
                        State::FormatSpec
                    } else {
                        State::Text
                    };
                } else {
                    argument.push(character);
                }
            }
            State::FormatSpec => {
                output.push(character);
                if character == '}' {
                    state = State::Text;
                }
            }
        }
    }

    if state != State::Text {
        return Err(syn::Error::new_spanned(
            message,
            "unclosed format placeholder",
        ));
    }

    Ok(LitStr::new(&output, message.span()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_unrelated_attributes() {
        let plain = quote! {
            #[display("{0}")]
            struct Value(String);
        };
        let annotated = quote! {
            #[error("unrelated")]
            #[display("{0}")]
            struct Value(#[error("unrelated")] #[source] #[from] #[backtrace] String);
        };
        assert_eq!(
            expand_from_parsed(syn::parse2(plain).unwrap(), DisplayOptions::None)
                .unwrap()
                .to_string(),
            expand_from_parsed(syn::parse2(annotated).unwrap(), DisplayOptions::None)
                .unwrap()
                .to_string()
        );
    }

    #[test]
    fn formats_enum_variants() {
        let input = quote! {
            enum Status {
                #[display("ok")]
                Ok,
                #[display("error: {0}")]
                Error(String),
                #[display("not found")]
                NotFound,
            }
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(output.contains("match self"));
        assert!(output.contains("Self :: Ok =>"));
        assert!(output.contains("Self :: Error (__foundations_field_0) =>"));
        assert!(output.contains("Self :: NotFound =>"));
    }

    #[test]
    fn formats_struct_fields() {
        let input = quote! {
            #[display("{name} is {age}")]
            struct Person {
                name: String,
                age: u32,
            }
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(output.contains("let Self { name , age } = self ;"));
        assert!(output.contains("let _ = name ; let _ = age ;"));
        assert!(output.contains("\"{name} is {age}\""));
    }

    #[test]
    fn formats_tuple_fields() {
        let input = quote! {
            #[display("{0} is {1}")]
            struct Point(f64, f64);
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(
            output.contains("let Self (__foundations_field_0 , __foundations_field_1) = self ;")
        );
        assert!(output.contains("\"{__foundations_field_0} is {__foundations_field_1}\""));
    }

    #[test]
    fn handles_raw_idents() {
        let input = quote! {
            #[display("{type} is {match}")]
            struct RawStruct {
                r#type: String,
                r#match: u32,
            }
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(output.contains("let Self { r#type , r#match } = self ;"));
        assert!(output.contains("let _ = r#type ; let _ = r#match ;"));
        assert!(output.contains("\"{type} is {match}\""));

        let input = quote! {
            enum RawEnum {
                #[display("type: {type}")]
                Variant { r#type: String },
            }
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(output.contains("Self :: Variant { r#type } =>"));
        assert!(output.contains("\"type: {type}\""));
    }

    #[test]
    fn preserves_format_spec_and_escaped_braces() {
        let input = quote! {
            #[display("{{ {0:.2} }}")]
            struct Value(f64);
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None)
            .unwrap()
            .to_string();
        assert!(output.contains("\"{{ {__foundations_field_0:.2} }}\""));
    }

    #[test]
    fn rejects_unclosed_placeholder() {
        let input = quote! {
            #[display("{0")]
            struct Value(String);
        };
        assert!(expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None).is_err());
    }

    #[test]
    fn rejects_mixed_implicit_and_explicit_placeholders() {
        for input in [
            quote! {
                #[display("{} {0}")]
                struct Value(String);
            },
            quote! {
                #[display("{0} {}")]
                struct Value(String);
            },
            quote! {
                #[display("{1} {2} {}")]
                struct Value(String);
            },
            quote! {
                #[display("{} {1}")]
                struct Value(String);
            },
        ] {
            assert!(expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None).is_err());
        }
    }

    #[test]
    fn rejects_unsupported_input() {
        for input in [
            quote! { struct Missing; },
            quote! { #[xyzzy("old spelling")] struct InvalidAttribute; },
            quote! { #[display(transparent)] struct Transparent(String); },
            quote! { #[display("{}", .0)] struct Expression(String); },
            quote! { #[display("one")] #[display("two")] struct Duplicate; },
            quote! { #[display("{0}")] struct FieldDisplay(#[display("field")] String); },
            quote! { #[display("{2}")] struct OutOfRange(String); },
            quote! { #[display("enum")] enum Outer { Variant } },
            quote! { enum MissingVariant { Variant } },
            quote! { union Unsupported { value: u32 } },
        ] {
            assert!(expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::None).is_err());
        }
    }

    #[test]
    fn error_derive_adds_error_impl() {
        let input = quote! {
            #[display("{0}")]
            struct Value(String);
        };
        let output = expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::ImplError)
            .unwrap()
            .to_string();
        assert!(output.contains("impl :: std :: fmt :: Display for Value"));
        assert!(output.contains("impl :: std :: error :: Error for Value"));
    }

    #[test]
    fn impl_blocks_are_automatically_derived() {
        let input = quote! {
            #[display("{0}")]
            struct Value(String);
        };
        let display_output =
            expand_from_parsed(syn::parse2(input.clone()).unwrap(), DisplayOptions::None)
                .unwrap()
                .to_string();
        assert!(display_output.contains("# [automatically_derived] impl :: std :: fmt :: Display"));
        assert!(
            !display_output.contains("# [automatically_derived] impl :: std :: error :: Error")
        );

        let error_output =
            expand_from_parsed(syn::parse2(input).unwrap(), DisplayOptions::ImplError)
                .unwrap()
                .to_string();
        let automatically_derived_count = error_output.matches("# [automatically_derived]").count();
        assert_eq!(automatically_derived_count, 2);
    }
}
