use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, LitStr, Path, parse_macro_input, parse_quote};

use crate::common::Result;

const DERIVE_TARGET_ERROR: &str = "LowCardinalityLabel can only be derived for a fieldless enum";

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_from_parsed(input)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

fn expand_from_parsed(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let crate_path = parse_crate_path(&input)?;
    let trait_path = quote!(#crate_path::telemetry::metrics::LowCardinalityLabel);
    let enum_name = &input.ident;
    let variants = match &input.data {
        Data::Enum(data) => &data.variants,
        Data::Struct(data) => {
            return Err(syn::Error::new(data.struct_token.span, DERIVE_TARGET_ERROR));
        }
        Data::Union(data) => {
            return Err(syn::Error::new(data.union_token.span, DERIVE_TARGET_ERROR));
        }
    };

    if variants.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            "LowCardinalityLabel requires at least one enum variant",
        ));
    }

    for variant in variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new(
                variant.fields.span(),
                "LowCardinalityLabel variants must not contain fields",
            ));
        }
    }

    let cardinality = variants.len();
    let match_arms = variants.iter().enumerate().map(|(index, variant)| {
        let variant_name = &variant.ident;
        quote!(Self::#variant_name => #index)
    });
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics #trait_path for #enum_name #type_generics #where_clause {
            const CARDINALITY: ::std::num::NonZeroUsize =
                ::std::num::NonZeroUsize::new(#cardinality).unwrap();

            fn index(&self) -> usize {
                match self {
                    #(#match_arms,)*
                }
            }
        }
    })
}

fn parse_crate_path(input: &DeriveInput) -> Result<Path> {
    let mut crate_path = None;

    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("low_cardinality_label"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("crate_path") {
                return Err(
                    meta.error("unsupported low_cardinality_label option; expected `crate_path`")
                );
            }

            if crate_path.is_some() {
                return Err(meta.error("duplicate `crate_path` option"));
            }

            let literal: LitStr = meta.value()?.parse()?;
            let path = literal.parse::<Path>().map_err(|error| {
                syn::Error::new(literal.span(), format!("invalid `crate_path`: {error}"))
            })?;
            crate_path = Some(path);
            Ok(())
        })?;
    }

    Ok(crate_path.unwrap_or_else(|| parse_quote!(::foundations)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_utils::code_str;
    use syn::parse_quote;

    #[test]
    fn expands_fieldless_enum_using_declaration_order() {
        let input = parse_quote! {
            enum Protocol {
                Tcp = 10,
                Udp = 2,
                Quic = 100,
            }
        };

        let actual = expand_from_parsed(input).unwrap().to_string();
        let expected = code_str! {
            impl ::foundations::telemetry::metrics::LowCardinalityLabel for Protocol {
                const CARDINALITY: ::std::num::NonZeroUsize =
                    ::std::num::NonZeroUsize::new(3usize).unwrap();

                fn index(&self) -> usize {
                    match self {
                        Self::Tcp => 0usize,
                        Self::Udp => 1usize,
                        Self::Quic => 2usize,
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn supports_crate_path_override_and_preserves_generics() {
        let input = parse_quote! {
            #[low_cardinality_label(crate_path = "::facade")]
            enum Generic<const N: usize>
            where
                [(); N]: Sized,
            {
                First,
                Second,
            }
        };

        let actual = expand_from_parsed(input).unwrap().to_string();
        let expected = code_str! {
            impl<const N: usize> ::facade::telemetry::metrics::LowCardinalityLabel for Generic<N>
            where
                [(); N]: Sized,
            {
                const CARDINALITY: ::std::num::NonZeroUsize =
                    ::std::num::NonZeroUsize::new(2usize).unwrap();

                fn index(&self) -> usize {
                    match self {
                        Self::First => 0usize,
                        Self::Second => 1usize,
                    }
                }
            }
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_non_enum_input() {
        let input = parse_quote! {
            struct Labels;
        };

        let error = expand_from_parsed(input).unwrap_err();
        assert_eq!(error.to_string(), DERIVE_TARGET_ERROR);
    }

    #[test]
    fn rejects_empty_enum() {
        let input = parse_quote! {
            enum Labels {}
        };

        let error = expand_from_parsed(input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "LowCardinalityLabel requires at least one enum variant"
        );
    }

    #[test]
    fn rejects_data_carrying_variant() {
        let input = parse_quote! {
            enum Labels {
                Empty,
                Data(u8),
            }
        };

        let error = expand_from_parsed(input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "LowCardinalityLabel variants must not contain fields"
        );
    }

    #[test]
    fn rejects_duplicate_crate_path() {
        let input = parse_quote! {
            #[low_cardinality_label(crate_path = "::first")]
            #[low_cardinality_label(crate_path = "::second")]
            enum Labels { One }
        };

        let error = expand_from_parsed(input).unwrap_err();
        assert_eq!(error.to_string(), "duplicate `crate_path` option");
    }

    #[test]
    fn rejects_unknown_option() {
        let input = parse_quote! {
            #[low_cardinality_label(other = "value")]
            enum Labels { One }
        };

        let error = expand_from_parsed(input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "unsupported low_cardinality_label option; expected `crate_path`"
        );
    }

    #[test]
    fn rejects_non_string_crate_path() {
        let input = parse_quote! {
            #[low_cardinality_label(crate_path = ::facade)]
            enum Labels { One }
        };

        assert!(expand_from_parsed(input).is_err());
    }
}
