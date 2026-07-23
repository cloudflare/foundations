use std::fmt;

use foundations_metrics_registry::proto::LabelPair;
use serde::Serialize;
use serde::ser::{Impossible, SerializeSeq, SerializeStruct, SerializeTuple, Serializer};

use super::LabelError;
use crate::validation::{NAME_REQUIREMENT, is_valid_name};

// Adapted from prometools' `serde::top::TopSerializer`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
pub(super) struct LabelSetSerializer;

impl Serializer for LabelSetSerializer {
    type Ok = Vec<LabelPair>;
    type Error = LabelError;
    type SerializeSeq = LabelSequenceSerializer;
    type SerializeTuple = LabelTupleSerializer;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = LabelPairSerializer;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(Vec::new())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(Vec::new())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(Vec::new())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(LabelPairSerializer {
            labels: Vec::with_capacity(len),
        })
    }

    fn serialize_bool(self, _value: bool) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_i8(self, _value: i8) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_i16(self, _value: i16) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_i32(self, _value: i32) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_i64(self, _value: i64) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_i128(self, _value: i128) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_u8(self, _value: u8) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_u16(self, _value: u16) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_u32(self, _value: u32) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_u64(self, _value: u64) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_u128(self, _value: u128) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_f32(self, _value: f32) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_f64(self, _value: f64) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_char(self, _value: char) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_str(self, _value: &str) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        Err(invalid_label_set())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(LabelSequenceSerializer {
            labels: Vec::with_capacity(len.unwrap_or_default()),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        if len != 2 {
            return Err(LabelError::new("label pairs must contain a name and value"));
        }

        Ok(LabelTupleSerializer {
            name: None,
            value: None,
        })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(invalid_label_set())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(invalid_label_set())
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

pub(super) struct LabelSequenceSerializer {
    labels: Vec<LabelPair>,
}

impl SerializeSeq for LabelSequenceSerializer {
    type Ok = Vec<LabelPair>;
    type Error = LabelError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let mut labels = value.serialize(LabelSetSerializer)?;
        if labels.len() != 1 {
            return Err(LabelError::new(
                "label sequences must contain name-value pairs",
            ));
        }
        self.labels
            .push(labels.pop().expect("one label was encoded"));
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.labels)
    }
}

pub(super) struct LabelTupleSerializer {
    name: Option<String>,
    value: Option<String>,
}

impl SerializeTuple for LabelTupleSerializer {
    type Ok = Vec<LabelPair>;
    type Error = LabelError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        if self.name.is_none() {
            let name = value.serialize(LabelValueSerializer)?;
            validate_label_name(&name)?;
            self.name = Some(name);
        } else if self.value.is_none() {
            self.value = Some(value.serialize(LabelValueSerializer)?);
        } else {
            return Err(LabelError::new(
                "label pairs must contain exactly one name and value",
            ));
        }

        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let name = self
            .name
            .ok_or_else(|| LabelError::new("label pair is missing its name"))?;
        let value = self
            .value
            .ok_or_else(|| LabelError::new("label pair is missing its value"))?;

        Ok(vec![LabelPair {
            name: Some(name),
            value: Some(value),
        }])
    }
}

// Adapted from prometools' `serde::top::StructSerializer`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
pub(super) struct LabelPairSerializer {
    labels: Vec<LabelPair>,
}

impl SerializeStruct for LabelPairSerializer {
    type Ok = Vec<LabelPair>;
    type Error = LabelError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        validate_label_name(key)?;
        self.labels.push(LabelPair {
            name: Some(key.to_owned()),
            value: Some(value.serialize(LabelValueSerializer)?),
        });
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.labels)
    }
}

// Adapted from prometools' `serde::value::ValueSerializer`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
struct LabelValueSerializer;

macro_rules! serialize_integer {
    ($($method:ident($ty:ty)),+ $(,)?) => {
        $(
            fn $method(self, value: $ty) -> Result<Self::Ok, Self::Error> {
                Ok(value.to_string())
            }
        )+
    };
}

impl Serializer for LabelValueSerializer {
    type Ok = String;
    type Error = LabelError;
    type SerializeSeq = Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    serialize_integer!(
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
    );

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        Ok(ryu::Buffer::new().format(value).to_owned())
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        Ok(ryu::Buffer::new().format(value).to_owned())
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_owned())
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(String::new())
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(name.to_owned())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(String::new())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn collect_str<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: fmt::Display + ?Sized,
    {
        Ok(value.to_string())
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(invalid_label_value("byte array"))
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        Err(invalid_label_value("newtype variant"))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(invalid_label_value("sequence"))
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(invalid_label_value("tuple"))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(invalid_label_value("tuple struct"))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(invalid_label_value("tuple variant"))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(invalid_label_value("map"))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(invalid_label_value("struct"))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(invalid_label_value("struct variant"))
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

// Adapted from prometools' `serde::top::check_key`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
fn validate_label_name(name: &str) -> Result<(), LabelError> {
    is_valid_name(name).then_some(()).ok_or_else(|| {
        LabelError::new(format!(
            "invalid metric label name {name:?}: expected {NAME_REQUIREMENT}"
        ))
    })
}

fn invalid_label_set() -> LabelError {
    LabelError::new("metric labels must serialize as a struct or unit")
}

fn invalid_label_value(kind: &str) -> LabelError {
    LabelError::new(format!("unsupported {kind} metric label value"))
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use crate::to_label_pairs;

    #[derive(Serialize)]
    #[serde(rename_all = "lowercase")]
    enum Method {
        Get,
    }

    #[derive(Serialize)]
    struct Labels<'a> {
        method: Method,
        status: u16,
        sampled: bool,
        ratio: f64,
        missing: Option<&'a str>,
        raw: &'a str,
    }

    #[test]
    fn serializes_supported_values_without_text_escaping() {
        let pairs = to_label_pairs(&Labels {
            method: Method::Get,
            status: 200,
            sampled: true,
            ratio: 1.0,
            missing: None,
            raw: "quote=\" slash=\\ newline=\n",
        })
        .unwrap();

        let values: Vec<_> = pairs
            .iter()
            .map(|pair| {
                (
                    pair.name.as_deref().unwrap(),
                    pair.value.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            values,
            [
                ("method", "get"),
                ("status", "200"),
                ("sampled", "true"),
                ("ratio", "1.0"),
                ("missing", ""),
                ("raw", "quote=\" slash=\\ newline=\n"),
            ]
        );
    }

    #[test]
    fn serializes_legacy_name_value_sequences() {
        let pairs = to_label_pairs(&vec![("trace_id", "abc"), ("span_id", "def")]).unwrap();
        let values: Vec<_> = pairs
            .iter()
            .map(|pair| {
                (
                    pair.name.as_deref().unwrap(),
                    pair.value.as_deref().unwrap(),
                )
            })
            .collect();

        assert_eq!(values, [("trace_id", "abc"), ("span_id", "def")]);
        assert!(to_label_pairs(&vec![("trace:id", "bad")]).is_err());
    }

    #[test]
    fn unit_is_an_empty_label_set() {
        assert!(to_label_pairs(&()).unwrap().is_empty());
    }

    #[test]
    fn rejects_empty_label_names() {
        #[derive(Serialize)]
        struct Invalid {
            #[serde(rename = "")]
            value: &'static str,
        }

        assert!(to_label_pairs(&Invalid { value: "x" }).is_err());
    }

    #[test]
    fn serializes_utf8_label_names() {
        #[derive(Serialize)]
        struct Labels {
            #[serde(rename = "trace.id λ\n\"")]
            value: &'static str,
        }

        let labels = to_label_pairs(&Labels { value: "x" }).unwrap();
        assert_eq!(labels[0].name.as_deref(), Some("trace.id λ\n\""));
    }

    #[test]
    fn rejects_compound_label_values() {
        #[derive(Serialize)]
        struct Invalid {
            values: Vec<u8>,
        }

        assert!(
            to_label_pairs(&Invalid {
                values: vec![1, 2, 3],
            })
            .is_err()
        );
    }
}
