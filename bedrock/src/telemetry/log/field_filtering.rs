use slog::{
    BorrowedKV, Drain, Key, Never, OwnedKV, OwnedKVList, Record, RecordStatic, Serializer, KV,
};
use std::fmt::Arguments;
use std::panic::RefUnwindSafe;

pub(crate) trait Filter {
    fn filter(&mut self, key: &Key) -> bool;
}

pub(crate) trait FilterFactory: Clone {
    type Filter: Filter;

    fn create_filter(&self) -> Self::Filter;
}

pub(crate) struct FieldFilteringDrain<F, D> {
    inner: D,
    filter_factory: F,
}

impl<F, D> FieldFilteringDrain<F, D> {
    pub(crate) fn new(inner: D, filter_factory: F) -> Self {
        Self {
            inner,
            filter_factory,
        }
    }
}

impl<F, D> Drain for FieldFilteringDrain<F, D>
where
    F: FilterFactory + Send + Sync + RefUnwindSafe + 'static,
    D: Drain<Err = Never>,
{
    type Ok = ();
    type Err = Never;

    fn log(&self, record: &Record, values: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let context_fields_kv = FieldFilteringKV {
            inner: values.clone(),
            filter_factory: self.filter_factory.clone(),
        };

        let record_fields_kv = FieldFilteringKV {
            inner: record.kv(),
            filter_factory: self.filter_factory.clone(),
        };

        let record_static = RecordStatic {
            location: record.location(),
            tag: record.tag(),
            level: record.level(),
        };

        let filtered_record =
            Record::new(&record_static, record.msg(), BorrowedKV(&record_fields_kv));

        self.inner
            .log(&filtered_record, &OwnedKV(context_fields_kv).into())
            .map(|_| ())
    }
}

struct FieldFilteringKV<K, F> {
    inner: K,
    filter_factory: F,
}

impl<K, F> KV for FieldFilteringKV<K, F>
where
    K: KV,
    F: FilterFactory,
{
    fn serialize(&self, record: &Record, inner: &mut dyn Serializer) -> slog::Result {
        let mut serializer = FieldFilteringSerializer {
            inner,
            filter: self.filter_factory.create_filter(),
        };

        self.inner.serialize(record, &mut serializer)
    }
}

struct FieldFilteringSerializer<'s, F> {
    inner: &'s mut dyn Serializer,
    filter: F,
}

macro_rules! filter {
    ( $self:ident.$fn:ident($key:expr, $val:expr) ) => {{
        if !$self.filter.filter(&$key) {
            return Ok(());
        }

        $self.inner.$fn($key, $val)
    }};
}

impl<'s, F: Filter> Serializer for FieldFilteringSerializer<'s, F> {
    fn emit_arguments(&mut self, key: Key, val: &Arguments) -> slog::Result {
        filter!(self.emit_arguments(key, val))
    }

    fn emit_usize(&mut self, key: Key, val: usize) -> slog::Result {
        filter!(self.emit_usize(key, val))
    }

    fn emit_isize(&mut self, key: Key, val: isize) -> slog::Result {
        filter!(self.emit_isize(key, val))
    }

    fn emit_bool(&mut self, key: Key, val: bool) -> slog::Result {
        filter!(self.emit_bool(key, val))
    }

    fn emit_char(&mut self, key: Key, val: char) -> slog::Result {
        filter!(self.emit_char(key, val))
    }

    fn emit_u8(&mut self, key: Key, val: u8) -> slog::Result {
        filter!(self.emit_u8(key, val))
    }

    fn emit_i8(&mut self, key: Key, val: i8) -> slog::Result {
        filter!(self.emit_i8(key, val))
    }

    fn emit_u16(&mut self, key: Key, val: u16) -> slog::Result {
        filter!(self.emit_u16(key, val))
    }

    fn emit_i16(&mut self, key: Key, val: i16) -> slog::Result {
        filter!(self.emit_i16(key, val))
    }

    fn emit_u32(&mut self, key: Key, val: u32) -> slog::Result {
        filter!(self.emit_u32(key, val))
    }

    fn emit_i32(&mut self, key: Key, val: i32) -> slog::Result {
        filter!(self.emit_i32(key, val))
    }

    fn emit_f32(&mut self, key: Key, val: f32) -> slog::Result {
        filter!(self.emit_f32(key, val))
    }

    fn emit_u64(&mut self, key: Key, val: u64) -> slog::Result {
        filter!(self.emit_u64(key, val))
    }

    fn emit_i64(&mut self, key: Key, val: i64) -> slog::Result {
        filter!(self.emit_i64(key, val))
    }

    fn emit_f64(&mut self, key: Key, val: f64) -> slog::Result {
        filter!(self.emit_f64(key, val))
    }

    #[cfg(integer128)]
    fn emit_u128(&mut self, key: Key, val: u128) -> slog::Result {
        filter!(self.emit_u128(key, val))
    }

    #[cfg(integer128)]
    fn emit_i128(&mut self, key: Key, val: i128) -> slog::Result {
        filter!(self.emit_i128(key, val))
    }

    fn emit_str(&mut self, key: Key, val: &str) -> slog::Result {
        filter!(self.emit_str(key, val))
    }

    fn emit_unit(&mut self, key: Key) -> slog::Result {
        if !self.filter.filter(&key) {
            return Ok(());
        }

        self.inner.emit_unit(key)
    }

    fn emit_none(&mut self, key: Key) -> slog::Result {
        if !self.filter.filter(&key) {
            return Ok(());
        }

        self.inner.emit_none(key)
    }
}
