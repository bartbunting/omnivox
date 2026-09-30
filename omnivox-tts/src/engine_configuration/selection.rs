use std::collections::BTreeSet;

/// Host-owned permission floors, independent of helper descriptors and session
/// preference ordering. Explicit permission applies only at an explicit stage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineSelectionPermissions {
    disabled: BTreeSet<String>,
    explicit_only: BTreeSet<String>,
}

impl EngineSelectionPermissions {
    pub fn new(
        disabled: BTreeSet<String>,
        external: BTreeSet<String>,
        automatic: &BTreeSet<String>,
    ) -> Self {
        Self {
            disabled,
            explicit_only: external.difference(automatic).cloned().collect(),
        }
    }

    pub fn disabled(&self, engine: &str) -> bool {
        self.disabled.contains(engine)
    }

    pub fn permits_automatic(&self, engine: &str) -> bool {
        !self.disabled(engine) && !self.explicit_only.contains(engine)
    }

    pub(crate) fn include_disabled(&mut self, engines: &[String]) {
        self.disabled.extend(engines.iter().cloned());
    }

    pub fn retained_bytes(&self) -> usize {
        self.disabled
            .iter()
            .chain(&self.explicit_only)
            .map(|id| id.len().saturating_add(std::mem::size_of::<String>()))
            .fold(0, usize::saturating_add)
    }
}
