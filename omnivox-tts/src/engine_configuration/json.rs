//! Build bounded JSON values while rejecting duplicate decoded keys.
use super::{ConfigurationError, Result, MAX_JSON_DEPTH};
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

pub(super) fn parse(bytes: &[u8], limit: usize) -> Result<Value> {
    parse_inner(bytes, limit, false)
}

/// Only punctuation character values may be null in public configuration.
/// Its typed parser checks that subtree; every other public field retains the
/// original no-null contract, including nullable internal Rust representations.
pub(super) fn parse_configuration(bytes: &[u8], limit: usize) -> Result<Value> {
    fn has_null(value: &Value, path: &mut Vec<String>) -> bool {
        if path == &["speech", "punctuation"] {
            return false;
        }
        match value {
            Value::Null => true,
            Value::Array(values) => values.iter().any(|value| has_null(value, path)),
            Value::Object(fields) => fields.iter().any(|(key, value)| {
                path.push(key.clone());
                let invalid = has_null(value, path);
                path.pop();
                invalid
            }),
            _ => false,
        }
    }
    let value = parse_inner(bytes, limit, true)?;
    if has_null(&value, &mut Vec::new()) {
        return Err(ConfigurationError::new(
            "JSON",
            "null is not allowed outside punctuation entries",
        ));
    }
    Ok(value)
}

/// Private complete records use required nullable fields. Public configuration
/// retains its separately validated null policy.
pub(super) fn parse_snapshot(bytes: &[u8], limit: usize) -> Result<Value> {
    parse_inner(bytes, limit, true)
}

fn parse_inner(bytes: &[u8], limit: usize, nullable: bool) -> Result<Value> {
    if bytes.len() > limit {
        return Err(ConfigurationError::new("JSON", "file exceeds byte limit"));
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let value = Node(0, nullable).deserialize(&mut reader).map_err(|_| {
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

struct Node(usize, bool);
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
    fn visit_unit<E: Error>(self) -> std::result::Result<Value, E> {
        if self.1 {
            Ok(Value::Null)
        } else {
            Err(E::custom("null is not allowed"))
        }
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
        while let Some(value) = sequence.next_element_seed(Node(self.0 + 1, self.1))? {
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
            let value = map.next_value_seed(Node(self.0 + 1, self.1))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
