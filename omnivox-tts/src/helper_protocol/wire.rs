//! Decode envelopes without losing duplicate or version-specific members.
//!
//! The body enums own their field sets; remove only the two envelope fields
//! before decoding them. Shared engine descriptors retain their existing schema.

use std::fmt;

use serde::de::{DeserializeOwned, Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::*;

// Retain the JSON tree while rejecting duplicate decoded names at every depth.
// Parsing straight into Value would silently discard duplicate object members.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("JSON without duplicate object members")
            }
            fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_bool<E: Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_i64<E: Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_u64<E: Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_f64<E: Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|v| UniqueValue(Value::Number(v)))
                    .ok_or_else(|| E::custom("nonfinite JSON number"))
            }
            fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_string<E: Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom("duplicate object member"));
                    }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

fn take<T: DeserializeOwned, E: Error>(object: &mut Map<String, Value>, key: &str) -> Result<T, E> {
    serde_json::from_value(
        object
            .remove(key)
            .ok_or_else(|| E::custom(format!("missing {key}")))?,
    )
    .map_err(E::custom)
}

fn envelope<'de, D: Deserializer<'de>, I: DeserializeOwned>(
    deserializer: D,
    unowned_error: bool,
) -> Result<(u16, I, Map<String, Value>), D::Error> {
    let UniqueValue(value) = UniqueValue::deserialize(deserializer)?;
    let Value::Object(mut fields) = value else {
        return Err(D::Error::custom("helper frame must be an object"));
    };
    if unowned_error
        && fields.get("type").and_then(Value::as_str) == Some("error")
        && !fields.contains_key("request_id")
    {
        // Older helpers may omit the ID when malformed input has no trusted owner.
        fields.insert("request_id".to_owned(), Value::Null);
    }
    let version = take(&mut fields, "protocol_version")?;
    let id = take(&mut fields, "request_id")?;
    Ok((version, id, fields))
}

fn reject_members<E: Error>(value: &Value, names: &[&str]) -> Result<(), E> {
    if value
        .as_object()
        .is_some_and(|object| names.iter().any(|name| object.contains_key(*name)))
    {
        return Err(E::custom(
            "member is not available in this helper protocol version",
        ));
    }
    Ok(())
}

impl<'de> Deserialize<'de> for HelperRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (protocol_version, request_id, fields) = envelope(deserializer, false)?;
        // Serde's internally tagged unit variants ignore extra members even
        // with deny_unknown_fields; check the payload-free operations here.
        if fields.len() != 1
            && matches!(
                fields.get("type").and_then(Value::as_str),
                Some("describe" | "ping" | "shutdown")
            )
        {
            return Err(D::Error::custom("unknown helper request member"));
        }

        if fields.get("type").and_then(Value::as_str) == Some("synthesize") {
            if protocol_version < HELPER_PROTOCOL_V2 && fields.contains_key("anchors") {
                return Err(D::Error::custom("anchors require helper protocol 2"));
            }
            if let Some(settings) = fields.get("settings") {
                if protocol_version < HELPER_PROTOCOL_V3 {
                    reject_members::<D::Error>(settings, &["pitch_range", "stress", "richness"])?;
                }
            }
            if let Some(Value::Array(anchors)) = fields.get("anchors") {
                for anchor in anchors {
                    if let Value::Object(members) = anchor {
                        if members
                            .keys()
                            .any(|key| !["id", "text_offset", "affinity"].contains(&key.as_str()))
                        {
                            return Err(D::Error::custom("unknown requested anchor member"));
                        }
                    }
                }
            }
        }
        let body = serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(Self {
            protocol_version,
            request_id,
            body,
        })
    }
}

impl<'de> Deserialize<'de> for HelperResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (protocol_version, request_id, fields) = envelope(deserializer, true)?;
        if fields.len() != 1
            && matches!(
                fields.get("type").and_then(Value::as_str),
                Some("pong" | "synthesis_cancelled" | "shutting_down")
            )
        {
            return Err(D::Error::custom("unknown helper response member"));
        }

        if protocol_version < HELPER_PROTOCOL_V5 {
            if let Some(Value::Array(markers)) = fields.get("markers") {
                for marker in markers {
                    reject_members::<D::Error>(marker, &["resolution"])?;
                }
            }
        }
        let body = serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(Self {
            protocol_version,
            request_id,
            body,
        })
    }
}
