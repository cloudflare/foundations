use super::{ArgAttrs, ArgMode, FnArg, FnAttrs, ItemFn, MacroArgs, Mod};
use crate::common::{Result, error, parse_attr_value, parse_meta_list};
use darling::FromMeta;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    AngleBracketedGenericArguments, Attribute, GenericArgument, PathArguments, Token, TraitBound,
    TraitBoundModifier, Type, TypeImplTrait, TypeParamBound, braced, parenthesized,
};

const IMPL_TRAIT_ERROR: &str = "Only `impl Into<T>` is allowed";

const FN_ATTR_ERROR: &str = "Only `#[cfg]`, `#[doc]`, `#[ctor]`, `#[optional]`, and `#[with_removal]` are allowed on functions";

const DUPLICATE_CTOR_ATTR_ERROR: &str = "Duplicate `#[ctor]` attribute";
const DUPLICATE_OPTIONAL_ATTR_ERROR: &str = "Duplicate `#[optional]` attribute";
const DUPLICATE_WITH_REMOVAL_ATTR_ERROR: &str = "Duplicate `#[with_removal]` attribute";
const DUPLICATE_SERDE_ATTR_ERROR: &str = "Duplicate `#[serde]` attribute";
const DUPLICATE_SERDE_AS_ATTR_ERROR: &str = "Duplicate `#[serde_as]` attribute";

const ARG_ATTR_ERROR: &str = "Only `#[serde]` and `#[serde_as]` are allowed on function arguments";
const WITH_REMOVAL_NO_ARGS_ERROR: &str =
    "`#[with_removal]` can only be used on functions with arguments";

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(Default::default());
        }

        let meta_list = parse_meta_list(&input)?;

        Ok(Self::from_list(&meta_list)?)
    }
}

impl Parse for Mod {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        let mod_token = input.parse()?;
        let ident = input.parse()?;
        let content;
        let _brace_token = braced!(content in input);
        let mut fns = vec![];

        while !content.is_empty() {
            fns.push(content.parse()?);
        }

        Ok(Self {
            attrs,
            vis,
            mod_token,
            ident,
            fns,
        })
    }
}

impl FnAttrs {
    fn from_attrs(attrs: Vec<Attribute>) -> Result<Self> {
        let mut cfg = vec![];
        let mut doc = "".to_owned();
        let mut ctor = None;
        let mut optional = None;
        let mut with_removal = None;

        for attr in attrs {
            let path = attr.path().get_ident().map(ToString::to_string);

            match path.as_deref() {
                Some("cfg") => cfg.push(attr),
                Some("doc") => doc.push_str(&String::from_meta(&attr.meta)?),
                Some("ctor") => {
                    if ctor.is_some() {
                        return error(&attr, DUPLICATE_CTOR_ATTR_ERROR);
                    }

                    ctor = Some(parse_attr_value(attr)?);
                }
                Some("optional") => {
                    if optional.is_some() {
                        return error(&attr, DUPLICATE_OPTIONAL_ATTR_ERROR);
                    }

                    optional = Some(bool::from_meta(&attr.meta).unwrap_or(true));
                }
                Some("with_removal") => {
                    if with_removal.is_some() {
                        return error(&attr, DUPLICATE_WITH_REMOVAL_ATTR_ERROR);
                    }

                    with_removal = Some(bool::from_meta(&attr.meta)?);
                }
                _ => return error(&attr, FN_ATTR_ERROR),
            }
        }

        Ok(Self {
            cfg,
            doc,
            ctor,
            optional: optional.unwrap_or(false),
            with_removal: with_removal.unwrap_or(false),
        })
    }
}

impl Parse for ItemFn {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = FnAttrs::from_attrs(input.call(Attribute::parse_outer)?)?;
        let vis = input.parse()?;
        let fn_token = input.parse()?;
        let ident = input.parse()?;
        let args_content;
        let paren_token = parenthesized!(args_content in input);
        let mut args = Punctuated::new();

        while !args_content.is_empty() {
            args.push_value(args_content.parse()?);

            if args_content.is_empty() {
                break;
            }

            args.push_punct(args_content.parse()?);
        }

        let arrow_token = input.parse()?;
        let ty = input.parse()?;
        let _semi_token = input.parse::<Token![;]>()?;

        if attrs.with_removal && args.is_empty() {
            return error(&paren_token.span, WITH_REMOVAL_NO_ARGS_ERROR);
        }

        Ok(ItemFn {
            attrs,
            vis,
            fn_token,
            ident,
            args,
            arrow_token,
            ty,
        })
    }
}

impl Parse for FnArg {
    fn parse(input: ParseStream) -> Result<Self> {
        fn parse_attrs(raw_attrs: Vec<Attribute>) -> Result<ArgAttrs> {
            let mut attrs = ArgAttrs::default();

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

        /// If the type is a reference with an eluded lifetime, then the argument should be cloned.
        /// If the type is `impl Into<Foo>`, then the Into trait should be used.
        /// Otherwise, use the value directly.
        fn arg_mode(ty: &Type) -> Result<ArgMode> {
            fn as_into_target(impl_: &TypeImplTrait) -> Result<&Type> {
                if impl_.bounds.len() != 1 {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                }

                let TypeParamBound::Trait(TraitBound {
                    modifier: TraitBoundModifier::None,
                    lifetimes: None,
                    path,
                    ..
                }) = &impl_.bounds[0]
                else {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                };

                if path.leading_colon.is_some() || path.segments.len() != 1 {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                }

                let segment = &path.segments[0];

                if segment.ident != "Into" {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                }

                let PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                    colon2_token: None,
                    args,
                    ..
                }) = &segment.arguments
                else {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                };

                if args.len() != 1 {
                    return error(&impl_, IMPL_TRAIT_ERROR);
                }

                match &args[0] {
                    GenericArgument::Type(ty) => Ok(ty),
                    _ => error(&impl_, IMPL_TRAIT_ERROR),
                }
            }

            Ok(match ty {
                Type::Reference(ref_) if ref_.lifetime.is_none() => {
                    ArgMode::Clone((*ref_.elem).clone())
                }
                Type::ImplTrait(impl_) => ArgMode::Into(as_into_target(impl_)?.clone()),
                ty => ArgMode::ByValue(ty.clone()),
            })
        }

        let ident = input.parse()?;
        let colon_token = input.parse()?;
        let ty = input.parse()?;
        let mode = arg_mode(&ty)?;

        Ok(Self {
            attrs,
            ident,
            colon_token,
            ty,
            mode,
        })
    }
}
