//! Helper-6 parameter messages, correlated evidence and catalogue assembly.
//!
//! These codecs cover new/extended operations. The parent session dispatcher
//! handles shared hello, PCM and terminal frames after version negotiation.
//! Parsing a message here grants no permission to send it to an old helper.
mod catalogue;
#[cfg(test)]
mod tests;
mod types;
mod validation;

use super::HelperProtocolError;
pub use catalogue::CatalogueAssembly;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
pub use types::*;

pub const PROTOCOL_VERSION: u16 = 6;
pub const MAX_PAGE_PARAMETERS: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub protocol_version: u16,
    pub request_id: u64,
    #[serde(flatten)]
    pub body: RequestBody,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    pub protocol_version: u16,
    pub request_id: u64,
    #[serde(flatten)]
    pub body: ResponseBody,
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (protocol_version, request_id, fields) = super::wire::envelope(d, false)?;
        let result = Self {
            protocol_version,
            request_id,
            body: serde_json::from_value(fields.into()).map_err(D::Error::custom)?,
        };
        result.validate().map_err(D::Error::custom)?;
        Ok(result)
    }
}
impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (protocol_version, request_id, fields) = super::wire::envelope(d, false)?;
        let result = Self {
            protocol_version,
            request_id,
            body: serde_json::from_value(fields.into()).map_err(D::Error::custom)?,
        };
        result.validate().map_err(D::Error::custom)?;
        Ok(result)
    }
}

fn invalid(field: &'static str) -> HelperProtocolError {
    HelperProtocolError::InvalidField(field)
}
fn require(ok: bool, field: &'static str) -> Result<(), HelperProtocolError> {
    if ok {
        Ok(())
    } else {
        Err(invalid(field))
    }
}
