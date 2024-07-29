use slog::{Drain, Never, OwnedKVList, Record, Serializer, KV};

use crate::telemetry::tracing;

pub(crate) struct TracingFilteringDrain<D: Drain<Err = Never>> {
    inner: D,
}

impl<D: Drain<Err = Never>> TracingFilteringDrain<D> {
    pub(crate) fn new(inner: D) -> Self {
        Self { inner }
    }
}

impl<D: Drain<Err = Never>> Drain for TracingFilteringDrain<D> {
    type Ok = ();
    type Err = D::Err;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        if tracing::rustracing_span().is_some() {
            self.inner.log(record, values).map(|_| ())
        } else {
            Ok(())
        }
    }
}

pub(crate) struct TracingDrain {}

impl TracingDrain {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Drain for TracingDrain {
    type Ok = ();
    type Err = Never;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let mut serializer = TracingValueSerializer::new();
        serializer.add_field("name".to_string(), record.module().to_string());
        serializer.add_field(
            "level".to_string(),
            record.level().as_short_str().to_string(),
        );
        serializer.add_field("msg".to_string(), record.msg().to_string());
        values.serialize(record, &mut serializer).unwrap();
        serializer.store();
        Ok(())
    }
}

struct TracingValueSerializer {
    fields: Vec<(String, String)>,
}

impl TracingValueSerializer {
    pub(crate) fn new() -> Self {
        // We know we're expecting at least name/level/msg next
        let fields = Vec::<(String, String)>::with_capacity(3);
        Self { fields }
    }

    pub(crate) fn add_field(&mut self, key: String, val: String) {
        self.fields.push((key, val));
    }

    pub(crate) fn store(self) {
        tracing::add_span_log_fields!(self.fields);
    }
}

impl Serializer for TracingValueSerializer {
    fn emit_arguments(&mut self, key: slog::Key, val: &std::fmt::Arguments<'_>) -> slog::Result {
        let value = val.to_string();
        self.fields.push((key.to_string(), value));
        Ok(())
    }
}
