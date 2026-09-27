//! Build a bounded JSON value while rejecting duplicate decoded keys and null.
use super::{ConfigurationError, Result, MAX_JSON_DEPTH};
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

pub(super) fn parse(bytes: &[u8], limit: usize) -> Result<Value> {
    if bytes.len() > limit {
        return Err(ConfigurationError::new("JSON", "file exceeds byte limit"));
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let value = Node(0).deserialize(&mut reader).map_err(|_| {
        ConfigurationError::new(
            "JSON",
            "invalid JSON, duplicate key, null or excessive nesting",
        )
    })?;
    reader
        .end()
        .map_err(|_| ConfigurationError::new("JSON", "trailing content"))?;
    Ok(value)
}

struct Node(usize);
impl<'de> DeserializeSeed<'de> for Node {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Node {
    type Value = Value;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded non-null JSON")
    }
    fn visit_bool<E: Error>(self, value: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: Error>(self, value: i64) -> std::result::Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: Error>(self, value: u64) -> std::result::Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: Error>(self, value: f64) -> std::result::Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid number"))
    }
    fn visit_str<E: Error>(self, value: &str) -> std::result::Result<Value, E> {
        Ok(Value::String(value.into()))
    }
    fn visit_string<E: Error>(self, value: String) -> std::result::Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<Value, A::Error> {
        if self.0 >= MAX_JSON_DEPTH {
            return Err(A::Error::custom("nesting limit"));
        }
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(Node(self.0 + 1))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
        if self.0 >= MAX_JSON_DEPTH {
            return Err(A::Error::custom("nesting limit"));
        }
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate decoded key"));
            }
            let value = map.next_value_seed(Node(self.0 + 1))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
