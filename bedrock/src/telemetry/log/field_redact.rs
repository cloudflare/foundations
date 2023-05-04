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
    use super::super::testing::{create_test_log, TestLogRecord};
    use slog::{warn, Level};

    #[test]
    fn redact_record_fields() {
        let (log, records) = create_test_log(vec!["key1".into(), "key3".into()]);

        warn!(log, "Hello world1"; "key1" => 42, "key2" => "foo");
        warn!(log, "Hello world2"; "key1" => "qux", "key3" => "foo");
        warn!(log, "Hello world3"; "key1" => "42", "key2" => "baz");

        assert_eq!(
            *records.read().unwrap(),
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

    #[test]
    fn redact_context_fields() {
        let (log, records) = create_test_log(vec!["key1".into(), "key4".into()]);

        let log = log.new(slog::o! {
           "key1" => 42, "key2" => "foo", "key3" => "beep boop"
        });

        let log = log.new(slog::o! {
           "key4" => "baz"
        });

        warn!(log, "Hello world");

        assert_eq!(
            *records.read().unwrap(),
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
