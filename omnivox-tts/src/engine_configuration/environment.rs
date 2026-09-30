use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::Command;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Private immutable native environment shared by launch and recovery.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchEnvironment(Arc<BTreeMap<OsString, OsString>>);

impl LaunchEnvironment {
    pub fn capture() -> Self {
        Self::from_inherited_variables(std::env::vars_os())
    }

    fn from_inherited_variables(variables: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        // Windows includes hidden drive-directory entries such as '=C:' in
        // vars_os(). They are process bookkeeping, not launch settings. Keep
        // the complete-record validator strict for explicitly supplied data.
        Self::from_variables(
            variables
                .into_iter()
                .filter(|(name, _)| inherited_launch_setting(name)),
        )
    }

    pub fn from_variables(variables: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self(Arc::new(variables.into_iter().collect()))
    }

    pub fn get(&self, key: &str) -> Option<&OsStr> {
        #[cfg(windows)]
        {
            // Windows variable names are case-insensitive. Keep the captured
            // spelling/bytes when passing the native environment to a child.
            self.0
                .iter()
                .find(|(name, _)| {
                    name.to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(key))
                })
                .map(|(_, value)| value.as_os_str())
        }
        #[cfg(not(windows))]
        self.0.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    pub fn apply(&self, command: &mut Command) {
        command.env_clear().envs(self.0.iter());
    }

    /// Change one host-owned setting using the launch platform's key semantics.
    /// Engine snapshots retain the original value; this returns a new record.
    pub fn with_variable(&self, key: &str, value: Option<OsString>) -> Self {
        let mut command = Command::new("unused-environment-projection");
        self.apply(&mut command);
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
        Self::from_variables(
            command
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned()))),
        )
    }

    pub(super) fn variables(&self) -> impl Iterator<Item = (&OsString, &OsString)> {
        self.0.iter()
    }
}

impl Serialize for LaunchEnvironment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.variables().collect::<Vec<_>>().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LaunchEnvironment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Existing retained owner records used UTF-8 maps. They remain readable
        // for inspection and package-retention decisions after an upgrade.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Native(Vec<(OsString, OsString)>),
            Legacy(BTreeMap<String, String>),
        }
        let variables = match Wire::deserialize(deserializer)
            .map_err(|_| serde::de::Error::custom("invalid private launch environment"))?
        {
            Wire::Native(variables) => variables,
            Wire::Legacy(variables) => variables
                .into_iter()
                // Historical Windows owner records captured vars_os directly,
                // including drive bookkeeping. Preserve their ordinary package
                // references while keeping native-pair records strict.
                .filter(|(key, _)| inherited_launch_setting(OsStr::new(key)))
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        };
        super::snapshot::validate_environment(&variables).map_err(serde::de::Error::custom)?;
        Ok(Self::from_variables(variables))
    }
}

fn inherited_launch_setting(name: &OsStr) -> bool {
    !cfg!(windows) || !name.as_encoded_bytes().starts_with(b"=")
}

impl fmt::Debug for LaunchEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LaunchEnvironment { <private> }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn inherited_drive_directories_are_omitted_without_changing_native_values() {
        use std::os::windows::ffi::OsStringExt;
        let private = OsString::from_wide(&[b'x' as u16, 0xd800]);
        let environment = LaunchEnvironment::from_inherited_variables([
            ("=C:".into(), "C:\\private working directory".into()),
            ("=D:".into(), "D:\\another directory".into()),
            ("PRIVATE".into(), private.clone()),
            ("EMPTY".into(), "".into()),
        ]);
        assert_eq!(environment.variables().count(), 2);
        assert_eq!(environment.get("PRIVATE"), Some(private.as_os_str()));
        assert_eq!(environment.get("EMPTY"), Some(OsStr::new("")));
        let decoded: LaunchEnvironment =
            serde_json::from_slice(&serde_json::to_vec(&environment).unwrap()).unwrap();
        assert_eq!(decoded, environment);
        let explicit = [(OsString::from("=C:"), OsString::from(r"C:\private"))];
        assert!(serde_json::from_slice::<LaunchEnvironment>(
            &serde_json::to_vec(&explicit).unwrap()
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn historical_windows_maps_keep_package_references_despite_drive_bookkeeping() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "=C:": r"C:\previous working directory",
            "OMNIVOX_PIPER_MODEL": r"C:\private package\voice.onnx",
            "EMPTY": "",
        }))
        .unwrap();
        let environment: LaunchEnvironment = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(environment.variables().count(), 2);
        assert_eq!(
            environment.get("OMNIVOX_PIPER_MODEL"),
            Some(OsStr::new(r"C:\private package\voice.onnx"))
        );
        assert_eq!(environment.get("EMPTY"), Some(OsStr::new("")));
        let restored: LaunchEnvironment =
            serde_json::from_slice(&serde_json::to_vec(&environment).unwrap()).unwrap();
        assert_eq!(restored, environment);
    }

    #[test]
    fn retained_legacy_maps_and_native_records_preserve_values() {
        let environment: LaunchEnvironment =
            serde_json::from_str(r#"{"EMPTY":"","PRIVATE":"value"}"#).unwrap();
        let bytes = serde_json::to_vec(&environment).unwrap();
        let restored: LaunchEnvironment = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, environment);
        let changed = restored
            .with_variable("PRIVATE", Some("replacement".into()))
            .with_variable("EMPTY", None);
        assert_eq!(changed.get("PRIVATE"), Some(OsStr::new("replacement")));
        assert_eq!(changed.get("EMPTY"), None);
        assert_eq!(environment.get("PRIVATE"), Some(OsStr::new("value")));
        assert_eq!(environment.get("EMPTY"), Some(OsStr::new("")));
        assert!(serde_json::from_str::<LaunchEnvironment>(r#"{"BAD=NAME":"value"}"#).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn inherited_windows_key_spelling_does_not_change_override_precedence() {
        let environment =
            LaunchEnvironment::from_variables([("Omnivox_Piper_Model".into(), "inherited".into())]);
        let changed = environment.with_variable("OMNIVOX_PIPER_MODEL", Some("candidate".into()));
        assert_eq!(changed.variables().count(), 1);
        assert_eq!(
            changed.get("OMNIVOX_PIPER_MODEL"),
            Some(OsStr::new("candidate"))
        );
        assert_eq!(
            changed
                .with_variable("OMNIVOX_PIPER_MODEL", None)
                .variables()
                .count(),
            0
        );
    }

    #[test]
    fn applying_a_snapshot_replaces_pending_environment_without_disclosure() {
        let environment = LaunchEnvironment::from_variables([
            ("OMNIVOX_PRIVATE_KEY".into(), "private-value".into()),
            ("OMNIVOX_EMPTY".into(), "".into()),
        ]);
        let mut command = Command::new("unused");
        command
            .env("UNRELATED", "must-not-leak")
            .env("OMNIVOX_PRIVATE_KEY", "new-value");
        environment.apply(&mut command);
        let variables: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(variables.len(), 2);
        assert_eq!(
            variables[OsStr::new("OMNIVOX_PRIVATE_KEY")],
            Some(OsStr::new("private-value"))
        );
        assert_eq!(variables[OsStr::new("OMNIVOX_EMPTY")], Some(OsStr::new("")));
        assert!(!format!("{environment:?}").contains("private-value"));
        assert!(!format!("{environment:?}").contains("OMNIVOX_PRIVATE_KEY"));
    }
}
