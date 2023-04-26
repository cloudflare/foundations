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
pub(super) struct FieldDedupFilterFactory;

impl FilterFactory for FieldDedupFilterFactory {
    type Filter = FieldDedupFilter;

    fn create_filter(&self) -> Self::Filter {
        Default::default()
    }
}

#[derive(Default)]
pub(super) struct FieldDedupFilter {
    seen_keys: HashSet<Key>,
}

impl Filter for FieldDedupFilter {
    #[inline]
    fn filter(&mut self, key: &Key) -> bool {
        self.seen_keys.insert(key.clone())
    }
}

#[cfg(test)]
mod tests {
    // NOTE: test log uses field dedup filter.
    use super::super::testing::{create_test_log, TestLogRecord};
    use slog::{warn, Level};

    #[test]
    fn remove_record_field_duplicates() {
        let (log, records) = create_test_log(Level::Warning, vec![]);

        warn!(log, "Hello world1"; "key1" => 42, "key2" => "foo", "key1" => "bar", "key1" => "baz");
        warn!(log, "Hello world2"; "key1" => "qux", "key1" => "foo");
        warn!(log, "Hello world3"; "key1" => "42", "key2" => "baz");

        assert_eq!(
            *records.read().unwrap(),
            vec![
                TestLogRecord {
                    message: "Hello world1".into(),
                    fields: vec![("key1".into(), "baz".into()), ("key2".into(), "foo".into())]
                },
                TestLogRecord {
                    message: "Hello world2".into(),
                    fields: vec![("key1".into(), "foo".into())]
                },
                TestLogRecord {
                    message: "Hello world3".into(),
                    fields: vec![("key2".into(), "baz".into()), ("key1".into(), "42".into())]
                }
            ]
        );
    }

    #[test]
    fn remove_context_field_duplicates() {
        let (log, records) = create_test_log(Level::Warning, vec![]);

        let log = log.new(slog::o! {
           "key1" => 42, "key2" => "foo", "key1" => "bar", "key1" => "baz", "key3" => "beep boop"
        });

        let log = log.new(slog::o! {
           "key1" => "42", "key2" => "baz"
        });

        warn!(log, "Hello world");

        assert_eq!(
            *records.read().unwrap(),
            vec![TestLogRecord {
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
