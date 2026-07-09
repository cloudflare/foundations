/// Metadata attached to a metric at registration time.
///
/// `#[non_exhaustive]` so fields can be added later on without breaking
/// [`register`](crate::register). Build it from [`default`](Self::default) plus
/// the setters, since downstream crates can't use a struct literal.
#[non_exhaustive]
#[derive(Clone, Default)]
pub struct RegistrationMetadata {
    /// Whether the metric is exported only when optional metrics are requested.
    pub optional: bool,

    /// Whether to suppress the service-name prefix for this metric
    ///
    /// The subsystem prefix stays; only the service-name prefix is skipped, and
    /// only when the service name is applied as a prefix rather than a label.
    pub unprefixed: bool,
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
}
