/// Metadata attached to a metric at registration time.
///
/// `#[non_exhaustive]` so fields can be added later on without breaking
/// [`register`](crate::register). Build it from [`default`](Self::default) plus
/// the setters, since downstream crates can't use a struct literal.
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct RegistrationMetadata {
    /// Whether the metric is exported only when optional metrics are requested.
    pub optional: bool,

    /// Whether to suppress the service-name prefix for this metric.
    ///
    /// With service name `api` and `MetricPrefix`, enabling this changes:
    ///
    /// ```text
    /// Before: api_requests_total 1.0
    /// After:  requests_total 1.0
    /// ```
    pub unprefixed: bool,

    /// Whether to suppress the service-name label for this metric.
    ///
    /// With service name `api` and `LabelWithName("service")`, enabling this changes:
    ///
    /// ```text
    /// Before: requests_total{service="api"} 1.0
    /// After:  requests_total 1.0
    /// ```
    ///
    /// Independent of [`unprefixed`](Self::unprefixed): a metric that opts out
    /// of the service-name prefix is still labelled with the service name when
    /// collection represents it as a label. Info metrics opt out of both.
    pub unlabeled: bool,
}

impl RegistrationMetadata {
    /// Sets [`optional`](Self::optional)
    #[must_use]
    pub fn optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }

    /// Sets [`unprefixed`](Self::unprefixed)
    #[must_use]
    pub fn unprefixed(mut self, unprefixed: bool) -> Self {
        self.unprefixed = unprefixed;
        self
    }

    /// Sets [`unlabeled`](Self::unlabeled)
    #[must_use]
    pub fn unlabeled(mut self, unlabeled: bool) -> Self {
        self.unlabeled = unlabeled;
        self
    }
}
