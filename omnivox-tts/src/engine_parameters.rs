//! Read-only, current-worker engine parameter discovery.
//!
//! The public and helper catalogue shapes deliberately share one typed contract.
//! Calling this API must not synthesize, load voices, recover or reconnect engines.
pub use crate::helper_protocol::parameters::{
    CatalogueQuery, CatalogueResult, CatalogueUnavailable,
};

#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("invalid parameter query: {0}")]
    Invalid(String),
    #[error("stale parameter catalogue: {0}")]
    Stale(String),
}

pub fn validate_query(query: &CatalogueQuery) -> Result<(), CatalogueError> {
    query
        .validate()
        .map_err(|e| CatalogueError::Invalid(e.to_string()))
}

pub fn unavailable(reason: CatalogueUnavailable, message: impl Into<String>) -> CatalogueResult {
    CatalogueResult::Unavailable {
        reason,
        message: message.into(),
    }
}
