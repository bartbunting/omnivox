//! Validated host preferences shared by configuration and speech preparation.

use serde::{Deserialize, Serialize};

/// Maximum whitespace-delimited words in one prepared synthesis call.
/// Zero must never turn a configured limit into unlimited synthesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct ChunkWordLimit(u16);

impl ChunkWordLimit {
    pub fn get(self) -> usize {
        usize::from(self.0)
    }
}

impl Default for ChunkWordLimit {
    fn default() -> Self {
        Self(15)
    }
}

impl TryFrom<u16> for ChunkWordLimit {
    type Error = &'static str;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=100).contains(&value) {
            Ok(Self(value))
        } else {
            Err("max_chunk_words must be an integer from 1 through 100")
        }
    }
}

impl From<ChunkWordLimit> for u16 {
    fn from(value: ChunkWordLimit) -> Self {
        value.0
    }
}
