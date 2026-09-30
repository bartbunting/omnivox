use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use super::{ConfigurationError, ConfigurationRoot, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Unix,
}

impl Platform {
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }

    pub fn validate_path(self, value: &str, field: &str) -> Result<()> {
        let valid = !value.is_empty()
            && value.len() <= 4096
            && !value.chars().any(char::is_control)
            && match self {
                Self::MacOs | Self::Unix => value.starts_with('/'),
                Self::Windows => windows_absolute(value),
            };
        if valid {
            Ok(())
        } else {
            Err(ConfigurationError::new(
                field,
                "expected a fully absolute native path without controls",
            ))
        }
    }

    /// Injectable environment allows testing all native root rules without
    /// changing the process environment or interpreting WSL as Windows.
    pub fn configuration_root(
        self,
        explicit: Option<&OsStr>,
        environment: impl Fn(&str) -> Option<OsString>,
    ) -> Result<ConfigurationRoot> {
        let (value, field, selected) = if let Some(value) = explicit {
            (value.to_owned(), "--config-dir", true)
        } else if let Some(value) = environment("OMNIVOX_CONFIG_DIR").filter(|v| !v.is_empty()) {
            (value, "OMNIVOX_CONFIG_DIR", true)
        } else {
            let (base, suffix) = match self {
                Self::Windows => ("APPDATA", "omnivox"),
                Self::MacOs => ("HOME", "Library/Application Support/Omnivox"),
                Self::Unix if environment("XDG_CONFIG_HOME").is_some_and(|v| !v.is_empty()) => {
                    ("XDG_CONFIG_HOME", "omnivox")
                }
                Self::Unix => ("HOME", ".config/omnivox"),
            };
            let value = environment(base).ok_or_else(|| {
                ConfigurationError::new(base, "required platform directory is missing")
            })?;
            let value = value
                .to_str()
                .ok_or_else(|| ConfigurationError::new(base, "directory is not UTF-8"))?;
            self.validate_path(value, base)?;
            // Do not use the test host's PathBuf rules to join a Windows path.
            let separator = if self == Self::Windows { '\\' } else { '/' };
            (
                OsString::from(format!(
                    "{}{separator}{suffix}",
                    value.trim_end_matches(['/', '\\'])
                )),
                base,
                false,
            )
        };
        let text = value
            .to_str()
            .ok_or_else(|| ConfigurationError::new(field, "directory is not UTF-8"))?;
        self.validate_path(text, field)?;
        Ok(ConfigurationRoot {
            path: PathBuf::from(value),
            explicit: selected,
        })
    }
}

fn windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    let separator = |byte| matches!(byte, b'/' | b'\\');
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && separator(bytes[2])
    {
        return true;
    }
    if bytes.len() < 5 || !separator(bytes[0]) || !separator(bytes[1]) {
        return false;
    }
    let mut components = value[2..].split(['/', '\\']);
    let server = components.next().unwrap_or_default();
    let share = components.next().unwrap_or_default();
    !server.is_empty()
        && !matches!(server, "." | ".." | "?")
        && !share.is_empty()
        && !matches!(share, "." | "..")
}
