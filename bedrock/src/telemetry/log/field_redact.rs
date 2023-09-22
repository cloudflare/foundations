use super::field_filtering::{Filter, FilterFactory};
use slog::Key;
use std::collections::HashSet;
use std::sync::Arc;

/// A log filter that removes redacted keys.
#[derive(Clone)]
pub(crate) struct FieldRedactFilterFactory {
    redacted_keys: Arc<HashSet<String>>,
}

impl FieldRedactFilterFactory {
    pub(crate) fn new(redacted_keys: Vec<String>) -> Self {
        Self {
            redacted_keys: Arc::new(redacted_keys.into_iter().collect()),
        }
    }
}

impl FilterFactory for FieldRedactFilterFactory {
    type Filter = FieldRedactFilter;

    fn create_filter(&self) -> Self::Filter {
        FieldRedactFilter {
            redacted_keys: Arc::clone(&self.redacted_keys),
        }
    }
}

pub(crate) struct FieldRedactFilter {
    redacted_keys: Arc<HashSet<String>>,
}

impl Filter for FieldRedactFilter {
    #[inline]
    fn filter(&mut self, key: &Key) -> bool {
        !self.redacted_keys.contains(*key)
    }
}

#[cfg(test)]
mod tests {
    // NOTE: test log uses field redact filter.
    use crate::telemetry::{log, log::TestLogRecord, TestTelemetryContext};
    use bedrock_macros::with_test_telemetry;
    use slog::Level;

    #[with_test_telemetry(test, crate_path = "crate", redact_key = "key1", redact_key = "key3")]
    fn redact_record_fields(ctx: TestTelemetryContext) {
        log::warn!("Hello world1"; "key1" => 42, "key2" => "foo");
        log::warn!("Hello world2"; "key1" => "qux", "key3" => "foo");
        log::warn!("Hello world3"; "key1" => "42", "key2" => "baz");

        assert_eq!(
            *ctx.log_records(),
            vec![
                TestLogRecord {
                    level: Level::Warning,
                    message: "Hello world1".into(),
                    fields: vec![("key2".into(), "foo".into())]
                },
                TestLogRecord {
                    level: Level::Warning,
                    message: "Hello world2".into(),
                    fields: vec![]
                },
                TestLogRecord {
                    level: Level::Warning,
                    message: "Hello world3".into(),
                    fields: vec![("key2".into(), "baz".into())]
                }
            ]
        );
    }

    #[with_test_telemetry(test, crate_path = "crate", redact_key = "key1", redact_key = "key4")]
    fn redact_context_fields(ctx: TestTelemetryContext) {
        log::add_fields! {
           "key1" => 42, "key2" => "foo", "key3" => "beep boop"
        }

        log::add_fields! {
           "key4" => "baz"
        }

        log::warn!("Hello world");

        assert_eq!(
            *ctx.log_records(),
            vec![TestLogRecord {
                level: Level::Warning,
                message: "Hello world".into(),
                fields: vec![
                    ("key3".into(), "beep boop".into()),
                    ("key2".into(), "foo".into()),
                ]
            },]
        );
    }
}
