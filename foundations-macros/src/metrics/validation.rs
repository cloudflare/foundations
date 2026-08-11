use syn::spanned::Spanned;
use syn::{Error, Expr, ExprLit, ExprStruct, Lit, Member, Result, UnOp};

/// Validates that histogram bucket values are strictly increasing.
/// Returns an error if the buckets are not valid.
pub(crate) fn validate_histogram_buckets(expr: &ExprStruct) -> Result<()> {
    let buckets = find_field(expr, "buckets")
        .ok_or_else(|| Error::new_spanned(expr, "histogram builder must have a 'buckets' field"))?;

    validate_bucket_array(buckets)
}

/// Validates literal classic buckets configured on a native histogram.
pub(crate) fn validate_native_histogram_buckets(expr: &ExprStruct) -> Result<()> {
    let Some(classic_buckets) = find_field(expr, "classic_buckets") else {
        return Ok(());
    };
    let Expr::Call(some) = classic_buckets else {
        return Ok(());
    };
    let is_some = if let Expr::Path(function) = &*some.func {
        function
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Some")
    } else {
        false
    };
    if !is_some || some.args.len() != 1 {
        return Ok(());
    }

    validate_bucket_array(&some.args[0])
}

fn find_field<'a>(expr: &'a ExprStruct, name: &str) -> Option<&'a Expr> {
    expr.fields.iter().find_map(|field| {
        if let Member::Named(ident) = &field.member
            && ident == name
        {
            Some(&field.expr)
        } else {
            None
        }
    })
}

fn validate_bucket_array(buckets: &Expr) -> Result<()> {
    let array_expr = match buckets {
        Expr::Reference(ref_expr) => match &*ref_expr.expr {
            Expr::Array(array) => array,
            _ => return Ok(()),
        },
        _ => return Ok(()),
    };

    let mut values = Vec::new();
    for elem in &array_expr.elems {
        let Some(value) = literal_bucket(elem)? else {
            return Ok(());
        };
        values.push(value);
    }

    if !values.is_empty() {
        let mut prev_value = values[0].0;
        for (i, (value, span)) in values.iter().enumerate().skip(1) {
            if *value <= prev_value {
                let message = format!(
                    "Histogram buckets must be strictly increasing. Found invalid bucket at position {i}: {value} <= {prev_value}"
                );
                return Err(Error::new(*span, message));
            }
            prev_value = *value;
        }
    }

    Ok(())
}

fn literal_bucket(expr: &Expr) -> Result<Option<(f64, proc_macro2::Span)>> {
    let (literal, sign) = match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Float(literal),
            ..
        }) => (literal, 1.0),
        Expr::Unary(unary) if matches!(unary.op, UnOp::Neg(_)) => {
            let Expr::Lit(ExprLit {
                lit: Lit::Float(literal),
                ..
            }) = &*unary.expr
            else {
                return Ok(None);
            };
            (literal, -1.0)
        }
        _ => return Ok(None),
    };

    Ok(Some((sign * literal.base10_parse::<f64>()?, expr.span())))
}
