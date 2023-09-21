use super::field_filtering::{Filter, FilterFactory};
use slog::Key;
use std::collections::HashSet;

/// A log filter that removes duplicate fields.
///
/// If there are two fields with the same key, only the first one is logged (the first occurrence
/// is the most recently added one).
///
/// Note that it's impossible to remove duplicates that are present both in context fields and
/// record fields. Those serialized separately and the order of serialization depends
/// on particular drain implementation.
#[derive(Clone)]
pub(crate) struct FieldDedupFilterFactory;

impl FilterFactory for FieldDedupFilterFactory {
    type Filter = FieldDedupFilter;

    fn create_filter(&self) -> Self::Filter {
        Default::default()
    }
}

#[derive(Default)]
pub(crate) struct FieldDedupFilter {
    seen_keys: HashSet<Key>,
}

impl Filter for FieldDedupFilter {
    #[inline]
    fn filter(&mut self, key: &Key) -> bool {
        self.seen_keys.insert(*key)
    }
}

#[cfg(test)]
mod tests {
    use slog::Level;
    // NOTE: test log uses field dedup filter.
    use super::super::testing::TestLogRecord;
    use crate::telemetry::{log, TestTelemetryContext};
    use bedrock_macros::with_test_telemetry;

    #[with_test_telemetry(test, crate_path = "crate")]
    fn remove_record_field_duplicates(ctx: TestTelemetryContext) {
        log::warn!("Hello world1"; "key1" => 42, "key2" => "foo", "key1" => "bar", "key1" => "baz");
        log::error!("Hello world2"; "key1" => "qux", "key1" => "foo");
        log::warn!("Hello world3"; "key1" => "42", "key2" => "baz");

        assert_eq!(
            *ctx.log_records(),
            vec![
                TestLogRecord {
                    level: Level::Warning,
                    message: "Hello world1".into(),
                    fields: vec![("key1".into(), "baz".into()), ("key2".into(), "foo".into())]
                },
                TestLogRecord {
                    level: Level::Error,
                    message: "Hello world2".into(),
                    fields: vec![("key1".into(), "foo".into())]
                },
                TestLogRecord {
                    level: Level::Warning,
                    message: "Hello world3".into(),
                    fields: vec![("key2".into(), "baz".into()), ("key1".into(), "42".into())]
                }
            ]
        );
    }

    #[with_test_telemetry(test, crate_path = "crate")]
    fn remove_context_field_duplicates(ctx: TestTelemetryContext) {
        log::add_fields! {
           "key1" => 42, "key2" => "foo", "key1" => "bar", "key1" => "baz", "key3" => "beep boop"
        }

        log::add_fields! {
           "key1" => "42", "key2" => "baz"
        }

        log::warn!("Hello world");

        assert_eq!(
            *ctx.log_records(),
            vec![TestLogRecord {
                level: Level::Warning,
                message: "Hello world".into(),
                fields: vec![
                    ("key2".into(), "baz".into()),
                    ("key1".into(), "42".into()),
                    ("key3".into(), "beep boop".into()),
                ]
            },]
        );
    }
}
