use super::{Field, FieldAttrs, MacroArgs, Struct, StructAttrs};
use crate::common::{error, parse_attr_value, parse_meta_list, Result};
use darling::FromMeta;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{braced, Attribute, LitStr};

const STRUCT_ATTR_ERROR: &str = "Only `#[cfg]` and `#[doc]` are allowed on structs";

const DUPLICATE_SERDE_ATTR_ERROR: &str = "Duplicate `#[serde]` attribute";
const DUPLICATE_SERDE_AS_ATTR_ERROR: &str = "Duplicate `#[serde_as]` attribute";

const ARG_ATTR_ERROR: &str = "Only `#[serde]` and `#[serde_as]` are allowed on struct fields";

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(Default::default());
        }

        let meta_list = parse_meta_list(&input)?;

        Ok(Self::from_list(&meta_list)?)
    }
}

impl Parse for Struct {
    fn parse(input: ParseStream) -> Result<Self> {
        fn parse_attrs(attrs: Vec<Attribute>) -> Result<StructAttrs> {
            let mut cfg = vec![];
            let mut doc = "".to_owned();

            for attr in attrs {
                let path = attr.path();

                if path.is_ident("cfg") {
                    cfg.push(attr);
                } else if path.is_ident("doc") {
                    doc.push_str(&parse_attr_value::<LitStr>(attr)?.value());
                } else {
                    return error(&attr, STRUCT_ATTR_ERROR);
                }
            }

            Ok(StructAttrs { cfg, doc })
        }

        let attrs = parse_attrs(input.call(Attribute::parse_outer)?)?;
        let vis = input.parse()?;
        let struct_token = input.parse()?;
        let ident = input.parse()?;
        let fields_content;
        let _brace_token = braced!(fields_content in input);
        let mut fields = Punctuated::new();

        while !fields_content.is_empty() {
            fields.push_value(fields_content.parse()?);

            if fields_content.is_empty() {
                break;
            }

            fields.push_punct(fields_content.parse()?);
        }

        Ok(Self {
            attrs,
            vis,
            struct_token,
            ident,
            fields,
        })
    }
}

impl Parse for Field {
    fn parse(input: ParseStream) -> Result<Self> {
        fn parse_attrs(raw_attrs: Vec<Attribute>) -> Result<FieldAttrs> {
            let mut attrs = FieldAttrs::default();

            for attr in raw_attrs {
                let path = attr.path();

                if path.is_ident("serde") {
                    if attrs.serde.is_some() {
                        return error(&attr, DUPLICATE_SERDE_ATTR_ERROR);
                    }

                    attrs.serde = Some(attr);
                } else if path.is_ident("serde_as") {
                    if attrs.serde_as.is_some() {
                        return error(&attr, DUPLICATE_SERDE_AS_ATTR_ERROR);
                    }

                    attrs.serde_as = Some(attr);
                } else {
                    return error(&attr, ARG_ATTR_ERROR);
                }
            }

            Ok(attrs)
        }

        let attrs = parse_attrs(input.call(Attribute::parse_outer)?)?;
        let vis = input.parse()?;
        let ident = input.parse()?;
        let colon_token = input.parse()?;
        let ty = input.parse()?;

        Ok(Self {
            attrs,
            vis,
            ident,
            colon_token,
            ty,
        })
    }
}
