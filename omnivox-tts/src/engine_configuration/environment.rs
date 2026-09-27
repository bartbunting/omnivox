use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::Command;
use std::sync::Arc;

/// Private immutable native environment shared by launch and recovery.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchEnvironment(Arc<BTreeMap<OsString, OsString>>);

impl LaunchEnvironment {
    pub fn capture() -> Self {
        Self::from_variables(std::env::vars_os())
    }

    pub fn from_variables(variables: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self(Arc::new(variables.into_iter().collect()))
    }

    pub fn get(&self, key: &str) -> Option<&OsStr> {
        self.0.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    pub fn apply(&self, command: &mut Command) {
        command.env_clear().envs(self.0.iter());
    }
}

impl fmt::Debug for LaunchEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LaunchEnvironment { <private> }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
