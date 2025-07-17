use syn::{Error, Expr, ExprLit, ExprStruct, Lit, Member, Result};

/// Validates that histogram bucket values are strictly increasing.
/// Returns an error if the buckets are not valid.
pub(crate) fn validate_histogram_buckets(expr: &ExprStruct) -> Result<()> {
    // Extract the buckets field from the HistogramBuilder struct
    let buckets_field = expr
        .fields
        .iter()
        .find(|field| {
            if let Member::Named(ident) = &field.member {
                ident == "buckets"
            } else {
                false
            }
        })
        .ok_or_else(|| Error::new_spanned(expr, "HistogramBuilder must have a 'buckets' field"))?;

    // Extract the array expression from the buckets field
    let array_expr = match &buckets_field.expr {
        Expr::Reference(ref_expr) => match &*ref_expr.expr {
            Expr::Array(array) => array,
            // For non-array literals (like variables or function calls), skip validation
            _ => return Ok(()),
        },
        // For non-reference expressions, skip validation
        _ => return Ok(()),
    };

    // Extract the f64 values from the array
    let mut values = Vec::new();
    for elem in &array_expr.elems {
        if let Expr::Lit(ExprLit {
            lit: Lit::Float(lit_float),
            ..
        }) = elem
        {
            let value = lit_float.base10_parse::<f64>()?;
            values.push((value, lit_float.span()));
        } else {
            // For non-float literals (like variables or expressions), skip validation
            return Ok(());
        }
    }

    // Check if the buckets are strictly increasing
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
