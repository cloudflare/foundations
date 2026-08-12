//! Choosing which wire format to serve a metrics scrape.
//!
//! Negotiation is deliberately separate from collection: [`negotiate`] only
//! reads an `Accept` header and [`collect_format`](super::collect_format) only
//! encodes, so a service exposing its own endpoint can reuse either half on its
//! own. Nothing here depends on the active metrics backend.

/// Content type of the text exposition with legacy name escaping.
const LEGACY_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[cfg(feature = "foundations-metrics-backend")]
use foundations_metrics::{OPENMETRICS_CONTENT_TYPE, PROTOBUF_CONTENT_TYPE};

#[cfg(not(feature = "foundations-metrics-backend"))]
const OPENMETRICS_CONTENT_TYPE: &str =
    "application/openmetrics-text; version=1.0.0; charset=utf-8; escaping=allow-utf-8";

#[cfg(not(feature = "foundations-metrics-backend"))]
const PROTOBUF_CONTENT_TYPE: &str =
    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited";

/// Wire format a scraper asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrapeFormat {
    /// Length-delimited Prometheus protobuf, the only format able to carry
    /// native histograms.
    Protobuf,

    /// OpenMetrics text, optionally permitted to quote UTF-8 names.
    Text {
        /// Whether the scraper accepts names quoted rather than escaped.
        utf8_names: bool,
    },
}

impl ScrapeFormat {
    /// The format assumed when a scraper expresses no usable preference.
    ///
    /// Every backend can produce it, so it is always safe to serve.
    pub const fn fallback() -> Self {
        Self::Text { utf8_names: false }
    }

    /// Content type describing what this format produces.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Protobuf => PROTOBUF_CONTENT_TYPE,
            Self::Text { utf8_names: true } => OPENMETRICS_CONTENT_TYPE,
            Self::Text { utf8_names: false } => LEGACY_CONTENT_TYPE,
        }
    }
}

/// Chooses the most preferred format that the caller can produce, given the
/// value of a request's `Accept` header.
///
/// `None` means the header ruled out every format on offer. An absent header
/// expresses no preference and yields [`ScrapeFormat::fallback`] instead.
///
/// The negotiated escaping is currently reported rather than enforced: the text
/// encoder quotes a name whenever that name requires it, regardless of what the
/// scraper asked for.
pub fn negotiate(accept: Option<&str>, allow_protobuf: bool) -> Option<ScrapeFormat> {
    let Some(accept) = accept else {
        return Some(ScrapeFormat::fallback());
    };

    let mut best: Option<(f32, ScrapeFormat)> = None;

    for range in accept.split(',') {
        let mut parts = range.split(';').map(str::trim);
        let Some(media_type) = parts.next().filter(|media| !media.is_empty()) else {
            continue;
        };

        let mut quality = 1.0f32;
        let (mut escaping, mut proto, mut encoding) = (None, None, None);

        for parameter in parts {
            let Some((name, value)) = parameter.split_once('=') else {
                continue;
            };

            let value = value.trim().trim_matches('"');
            let name = name.trim();

            if name.eq_ignore_ascii_case("q") {
                quality = match value.parse::<f32>() {
                    Ok(parsed) if (0.0..=1.0).contains(&parsed) => parsed,
                    _ => 0.0,
                };
            } else if name.eq_ignore_ascii_case("escaping") {
                escaping = Some(value);
            } else if name.eq_ignore_ascii_case("proto") {
                proto = Some(value);
            } else if name.eq_ignore_ascii_case("encoding") {
                encoding = Some(value);
            }
        }

        // `q=0` refuses a format outright rather than ranking it last.
        if quality <= 0.0 {
            continue;
        }

        let format = if media_type.eq_ignore_ascii_case("application/vnd.google.protobuf") {
            // Only delimited streams of this message type are produced, and only
            // when protobuf can carry the whole exposition.
            if allow_protobuf
                && proto.is_some_and(|proto| proto == "io.prometheus.client.MetricFamily")
                && encoding.is_some_and(|encoding| encoding.eq_ignore_ascii_case("delimited"))
            {
                ScrapeFormat::Protobuf
            } else {
                continue;
            }
        } else if media_type.eq_ignore_ascii_case("application/openmetrics-text") {
            ScrapeFormat::Text {
                utf8_names: escaping
                    .is_some_and(|escaping| escaping.eq_ignore_ascii_case("allow-utf-8")),
            }
        } else if media_type.eq_ignore_ascii_case("text/plain") || media_type == "*/*" {
            ScrapeFormat::Text { utf8_names: false }
        } else {
            continue;
        };

        // Highest quality wins; ties keep the earliest listed.
        if best.is_none_or(|(best_quality, _)| quality > best_quality) {
            best = Some((quality, format));
        }
    }

    best.map(|(_, format)| format)
}

/// Negotiates a format, serving [`ScrapeFormat::fallback`] with a warning when
/// the `Accept` header rules out everything on offer.
///
/// This is what the telemetry server's `/metrics` route does; callers wanting to
/// reject an unsatisfiable header with `406 Not Acceptable` instead should match
/// on [`negotiate`] directly.
pub fn negotiate_or_fallback(accept: Option<&str>, allow_protobuf: bool) -> ScrapeFormat {
    negotiate(accept, allow_protobuf).unwrap_or_else(|| {
        let fallback = ScrapeFormat::fallback();

        // Only a header that was present can rule everything out, so the
        // default here stands in for a case `negotiate` never reports.
        report_unsatisfiable_accept(accept.unwrap_or_default(), fallback.content_type());

        fallback
    })
}

/// Warns that a scrape's `Accept` header ruled out every available format.
///
/// Not routed through
/// [`report_nonfatal_collect_error`](super::report_nonfatal_collect_error),
/// because collection itself succeeded and the actionable detail is the header.
fn report_unsatisfiable_accept(accept: &str, served: &str) {
    #[cfg(feature = "logging")]
    crate::telemetry::log::warn!(
        "no requested metrics format can be served, responding with the fallback instead";
        "accept" => accept,
        "served" => served,
    );

    #[cfg(not(feature = "logging"))]
    eprintln!(
        "no requested metrics format can be served, responding with the fallback instead: \
         accept={accept:?} served={served:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: Option<ScrapeFormat> = Some(ScrapeFormat::Text { utf8_names: false });
    const TEXT_UTF8: Option<ScrapeFormat> = Some(ScrapeFormat::Text { utf8_names: true });
    const PROTOBUF: Option<ScrapeFormat> = Some(ScrapeFormat::Protobuf);

    /// What Prometheus sends unless configured to prefer protobuf.
    const PROMETHEUS_DEFAULT: &str = "application/openmetrics-text;version=1.0.0;q=0.5,\
                                      text/plain;version=0.0.4;q=0.4,*/*;q=0.1";

    const PROTOBUF_PREFERRED: &str = "application/vnd.google.protobuf;\
                                      proto=io.prometheus.client.MetricFamily;\
                                      encoding=delimited;q=0.5,\
                                      application/openmetrics-text;version=1.0.0;q=0.4";

    /// Delimited protobuf and nothing else: no text range, no `*/*`.
    const PROTOBUF_ONLY: &str = "application/vnd.google.protobuf;\
                                 proto=io.prometheus.client.MetricFamily;encoding=delimited";

    #[test]
    fn absent_header_falls_back_to_legacy_text() {
        assert_eq!(negotiate(None, true), TEXT);
    }

    #[test]
    fn prometheus_default_accept_selects_text() {
        assert_eq!(negotiate(Some(PROMETHEUS_DEFAULT), true), TEXT);
    }

    #[test]
    fn utf8_escaping_is_detected() {
        let accept =
            "application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.5,*/*;q=0.1";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn delimited_protobuf_wins_when_preferred() {
        assert_eq!(negotiate(Some(PROTOBUF_PREFERRED), true), PROTOBUF);
    }

    #[test]
    fn protobuf_without_delimited_encoding_is_not_offered() {
        let accept = "application/vnd.google.protobuf;\
                      proto=io.prometheus.client.MetricFamily;q=0.9,text/plain;q=0.1";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn zero_quality_refuses_a_format() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=0,text/plain;q=0.4";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn zero_quality_on_the_only_range_matches_nothing() {
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=0"), true),
            None
        );
    }

    #[test]
    fn malformed_quality_refuses_a_format() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=garbage,text/plain;q=0.4";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn parameters_tolerate_whitespace_case_and_quoting() {
        let accept =
            "  APPLICATION/OpenMetrics-Text ; Version=1.0.0 ; Escaping=\"Allow-UTF-8\" ; Q=0.7 ";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn ties_keep_the_earliest_listed() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=0.5,text/plain;q=0.5";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn protobuf_is_withheld_when_unavailable() {
        assert_eq!(negotiate(Some(PROTOBUF_PREFERRED), false), TEXT);
    }

    #[test]
    fn withholding_protobuf_leaves_text_negotiation_untouched() {
        let utf8 = "application/openmetrics-text;escaping=allow-utf-8;q=0.9,text/plain;q=0.1";

        assert_eq!(negotiate(Some(utf8), false), TEXT_UTF8);
        assert_eq!(negotiate(Some(PROMETHEUS_DEFAULT), false), TEXT);
        assert_eq!(negotiate(None, false), TEXT);
    }

    #[test]
    fn protobuf_only_matches_nothing_when_unavailable() {
        assert_eq!(negotiate(Some(PROTOBUF_ONLY), false), None);
    }

    #[test]
    fn protobuf_only_is_served_when_available() {
        assert_eq!(negotiate(Some(PROTOBUF_ONLY), true), PROTOBUF);
    }

    #[test]
    fn unservable_media_types_match_nothing() {
        assert_eq!(negotiate(Some("application/json,text/html"), true), None);
    }

    #[test]
    fn malformed_quality_on_the_only_range_matches_nothing() {
        let accept = "application/vnd.google.protobuf;\
                      proto=io.prometheus.client.MetricFamily;encoding=delimited;q=";

        assert_eq!(negotiate(Some(accept), true), None);
    }

    #[test]
    fn unrankable_quality_matches_nothing() {
        for weight in [
            "nan", "NaN", "+nan", "inf", "infinity", "-inf", "2", "1e3", "-0.5",
        ] {
            let accept = format!("application/openmetrics-text;q={weight}");

            assert_eq!(
                negotiate(Some(&accept), true),
                None,
                "q={weight} should invalidate the range"
            );
        }
    }

    #[test]
    fn unrankable_quality_does_not_mask_a_later_range() {
        let accept = format!("application/openmetrics-text;q=nan,{PROTOBUF_PREFERRED}");

        assert_eq!(negotiate(Some(&accept), true), PROTOBUF);
    }

    #[test]
    fn out_of_range_quality_does_not_outrank_the_maximum() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=5,text/plain;q=1.0";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn quality_bounds_are_accepted() {
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=1.000"), true),
            TEXT
        );
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=0.001"), true),
            TEXT
        );
    }

    /// Each content type must match the encoder that produces that format.
    #[cfg(feature = "foundations-metrics-backend")]
    #[test]
    fn content_types_come_from_the_encoders() {
        assert_eq!(
            ScrapeFormat::Protobuf.content_type(),
            foundations_metrics::PROTOBUF_CONTENT_TYPE
        );
        assert_eq!(
            ScrapeFormat::Text { utf8_names: true }.content_type(),
            foundations_metrics::OPENMETRICS_CONTENT_TYPE
        );
    }
}

/// Covers the warning emitted when an `Accept` header cannot be satisfied.
#[cfg(all(test, feature = "logging"))]
mod fallback_logging_tests {
    use super::*;
    use crate::telemetry::TelemetryContext;

    /// Matches nothing on offer whether or not protobuf is available.
    const UNSERVABLE: &str = "application/json,text/html";

    #[test]
    fn unsatisfiable_accept_is_served_as_text_with_a_warning() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        assert_eq!(
            negotiate_or_fallback(Some(UNSERVABLE), true),
            ScrapeFormat::fallback()
        );

        let records = ctx.log_records();
        let warning = records
            .iter()
            .find(|record| record.message.contains("no requested metrics format"))
            .unwrap_or_else(|| panic!("falling back should warn: {records:?}"));

        assert_eq!(warning.level, slog::Level::Warning);
        assert!(
            warning
                .fields
                .contains(&("accept".to_owned(), UNSERVABLE.to_owned())),
            "the warning should name the header that could not be satisfied: {:?}",
            warning.fields
        );
    }
}
