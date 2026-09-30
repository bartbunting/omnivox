//! Host punctuation tables, resolved before engines are constructed.
//!
//! Missing entries and explicit null both preserve the original scalar. Spoken
//! names contain no padding: text preparation supplies boundary spaces once.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::state::PunctuationLevel;

pub const MAX_PUNCTUATION_ENTRIES: usize = 512;
pub const MAX_PUNCTUATION_NAME_BYTES: usize = 64;

pub type PunctuationTable = BTreeMap<char, Option<String>>;

pub const MAX_PUNCTUATION_PROFILES: usize = 32;

/// One fully resolved immutable profile; inheritance is restricted to built-ins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PunctuationProfile {
    pub fallback: PunctuationLevel,
    pub table: PunctuationTable,
}

pub fn valid_profile_id(id: &str) -> bool {
    !matches!(id, "none" | "some" | "all")
        && !id.is_empty()
        && id.len() <= 32
        && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

/// Fully resolved tables. Deserialization requires all three levels; sparse
/// public configuration is merged separately before capturing a startup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PunctuationTables {
    pub none: PunctuationTable,
    pub some: PunctuationTable,
    pub all: PunctuationTable,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, PunctuationProfile>,
}

impl Default for PunctuationTables {
    fn default() -> Self {
        let mut tables = Self::legacy();
        tables.add_defaults(UNICODE_DEFAULTS);
        tables
    }
}

impl PunctuationTables {
    /// Original ASCII behavior, retained by historical startup schemas 1–5.
    pub fn legacy() -> Self {
        let mut tables = Self {
            none: BTreeMap::new(),
            some: BTreeMap::new(),
            all: BTreeMap::new(),
            profiles: BTreeMap::new(),
        };
        tables.add_defaults(ASCII_DEFAULTS);
        tables
    }

    fn add_defaults(&mut self, entries: &[(char, &str, PunctuationLevel)]) {
        for &(character, name, minimum) in entries {
            for level in [
                PunctuationLevel::None,
                PunctuationLevel::Some,
                PunctuationLevel::All,
            ] {
                let speak = match level {
                    PunctuationLevel::None => minimum == PunctuationLevel::None,
                    PunctuationLevel::Some => minimum != PunctuationLevel::All,
                    PunctuationLevel::All => true,
                };
                self.table_mut(level)
                    .insert(character, speak.then(|| name.to_owned()));
            }
        }
    }

    pub fn table_mut(&mut self, level: PunctuationLevel) -> &mut PunctuationTable {
        match level {
            PunctuationLevel::None => &mut self.none,
            PunctuationLevel::Some => &mut self.some,
            PunctuationLevel::All => &mut self.all,
        }
    }

    pub fn table(&self, level: PunctuationLevel) -> &PunctuationTable {
        match level {
            PunctuationLevel::None => &self.none,
            PunctuationLevel::Some => &self.some,
            PunctuationLevel::All => &self.all,
        }
    }

    pub fn spoken_name(&self, character: char, level: PunctuationLevel) -> Option<&str> {
        self.table(level).get(&character).and_then(Option::as_deref)
    }

    /// Validate public merges and complete private snapshots identically.
    /// Errors identify the rule, never user-provided text.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.profiles.len() > MAX_PUNCTUATION_PROFILES
            || self.profiles.keys().any(|id| !valid_profile_id(id))
        {
            return Err("invalid or excessive punctuation profile IDs");
        }
        for table in [&self.none, &self.some, &self.all]
            .into_iter()
            .chain(self.profiles.values().map(|profile| &profile.table))
        {
            if table.len() > MAX_PUNCTUATION_ENTRIES {
                return Err("too many punctuation entries");
            }
            for (character, name) in table {
                if character.is_control() || character.is_whitespace() {
                    return Err("punctuation key must be a non-whitespace, non-control scalar");
                }
                if let Some(name) = name {
                    if name.is_empty()
                        || name.len() > MAX_PUNCTUATION_NAME_BYTES
                        || name.trim() != name
                        || name.chars().any(char::is_control)
                    {
                        return Err("punctuation name must be 1–64 UTF-8 bytes without controls or edge whitespace");
                    }
                }
            }
        }
        Ok(())
    }
}

const ASCII_DEFAULTS: &[(char, &str, PunctuationLevel)] = &[
    ('!', "bang", PunctuationLevel::Some),
    ('"', "quote", PunctuationLevel::Some),
    ('#', "pound", PunctuationLevel::Some),
    ('$', "dollar", PunctuationLevel::None),
    ('%', "percent", PunctuationLevel::None),
    ('&', "ampersand", PunctuationLevel::All),
    ('\'', "apostrophe", PunctuationLevel::All),
    ('(', "left paren", PunctuationLevel::Some),
    (')', "right paren", PunctuationLevel::Some),
    ('*', "star", PunctuationLevel::Some),
    ('+', "plus", PunctuationLevel::Some),
    (',', "comma", PunctuationLevel::All),
    ('-', "dash", PunctuationLevel::Some),
    ('.', "dot", PunctuationLevel::All),
    ('/', "slash", PunctuationLevel::Some),
    (':', "colon", PunctuationLevel::Some),
    (';', "semicolon", PunctuationLevel::Some),
    ('<', "less than", PunctuationLevel::Some),
    ('=', "equals", PunctuationLevel::Some),
    ('>', "greater than", PunctuationLevel::Some),
    ('?', "question", PunctuationLevel::All),
    ('@', "at", PunctuationLevel::All),
    ('[', "left bracket", PunctuationLevel::All),
    ('\\', "backslash", PunctuationLevel::Some),
    (']', "right bracket", PunctuationLevel::All),
    ('^', "caret", PunctuationLevel::Some),
    ('_', "underline", PunctuationLevel::All),
    ('`', "backquote", PunctuationLevel::Some),
    ('{', "left brace", PunctuationLevel::All),
    ('|', "pipe", PunctuationLevel::All),
    ('}', "right brace", PunctuationLevel::All),
    ('~', "tilde", PunctuationLevel::Some),
];

// Common typographic forms. Unlisted Unicode remains available for prosody;
// users may define any additional non-whitespace, non-control scalar.
const UNICODE_DEFAULTS: &[(char, &str, PunctuationLevel)] = &[
    ('‘', "apostrophe", PunctuationLevel::All),
    ('’', "apostrophe", PunctuationLevel::All),
    ('ʼ', "apostrophe", PunctuationLevel::All),
    ('‚', "low single quote", PunctuationLevel::All),
    ('‛', "reversed single quote", PunctuationLevel::All),
    ('“', "quote", PunctuationLevel::Some),
    ('”', "quote", PunctuationLevel::Some),
    ('„', "low double quote", PunctuationLevel::Some),
    ('‟', "reversed double quote", PunctuationLevel::Some),
    ('«', "left guillemet", PunctuationLevel::Some),
    ('»', "right guillemet", PunctuationLevel::Some),
    ('‹', "left single guillemet", PunctuationLevel::All),
    ('›', "right single guillemet", PunctuationLevel::All),
    ('‐', "hyphen", PunctuationLevel::Some),
    ('‑', "nonbreaking hyphen", PunctuationLevel::Some),
    ('‒', "figure dash", PunctuationLevel::Some),
    ('–', "en dash", PunctuationLevel::Some),
    ('—', "em dash", PunctuationLevel::Some),
    ('―', "horizontal bar", PunctuationLevel::Some),
    ('…', "ellipsis", PunctuationLevel::All),
    ('•', "bullet", PunctuationLevel::All),
    ('·', "middle dot", PunctuationLevel::All),
    ('′', "prime", PunctuationLevel::All),
    ('″', "double prime", PunctuationLevel::All),
    ('−', "minus", PunctuationLevel::Some),
];
