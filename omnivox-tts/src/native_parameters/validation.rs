use std::collections::BTreeSet;

use crate::contracts::VoiceSelector;
use crate::voice_choices::Adjustment;

use super::*;

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn require(condition: bool, reason: &'static str) -> Result<(), ParameterError> {
    if condition {
        Ok(())
    } else {
        Err(ParameterError::Invalid(reason))
    }
}

impl NativeValue {
    pub fn validate(&self) -> Result<(), ParameterError> {
        match self {
            Self::Number(n) => require(n.is_finite(), "nonfinite native value"),
            Self::Enum(s) => require(identifier(s), "invalid enum identifier"),
            _ => Ok(()),
        }
    }
}

impl ValueType {
    pub fn validate(&self) -> Result<(), ParameterError> {
        match self {
            Self::Integer {
                minimum,
                maximum,
                step,
            } => require(
                minimum <= maximum && *step > 0,
                "invalid integer constraints",
            ),
            Self::Number {
                minimum,
                maximum,
                step,
            } => require(
                minimum.is_finite()
                    && maximum.is_finite()
                    && step.is_finite()
                    && minimum <= maximum
                    && *step > 0.0,
                "invalid number constraints",
            ),
            Self::Boolean {} => Ok(()),
            Self::Enum { choices } => {
                require(
                    !choices.is_empty() && choices.len() <= MAX_ENUM_CHOICES,
                    "invalid enum size",
                )?;
                let mut seen = BTreeSet::new();
                for choice in choices {
                    require(
                        identifier(&choice.value)
                            && text(&choice.label, 128)
                            && seen.insert(&choice.value),
                        "invalid or duplicate enum choice",
                    )?;
                }
                Ok(())
            }
        }
    }

    pub fn accepts(&self, value: &NativeValue) -> bool {
        if self.validate().is_err() || value.validate().is_err() {
            return false;
        }
        match (self, value) {
            (
                Self::Integer {
                    minimum, maximum, ..
                },
                NativeValue::Integer(v),
            ) => (minimum..=maximum).contains(&v),
            (
                Self::Number {
                    minimum, maximum, ..
                },
                NativeValue::Number(v),
            ) => (minimum..=maximum).contains(&v),
            (
                Self::Number {
                    minimum, maximum, ..
                },
                NativeValue::Integer(v),
            ) => (*minimum..=*maximum).contains(&(*v as f64)),
            (Self::Boolean {}, NativeValue::Boolean(_)) => true,
            (Self::Enum { choices }, NativeValue::Enum(v)) => choices.iter().any(|c| c.value == *v),
            _ => false,
        }
    }
}

impl CatalogueIdentity {
    pub fn validate(&self) -> Result<(), ParameterError> {
        require(
            identifier(&self.schema_id) && identifier(&self.profile_id),
            "invalid schema/profile ID",
        )?;
        require(
            self.runtime_generation != 0
                && self.catalogue_revision.len() == 64
                && self
                    .catalogue_revision
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid catalogue revision or runtime generation",
        )
    }
}

impl ParameterDescriptor {
    pub fn validate(&self) -> Result<(), ParameterError> {
        require(
            identifier(&self.id) && identifier(&self.group),
            "invalid parameter/group ID",
        )?;
        require(
            text(&self.label, 128) && text(&self.help, 1024),
            "invalid label/help",
        )?;
        require(self.unit.as_deref().is_none_or(identifier), "invalid unit")?;
        self.value_type.validate()?;
        let supported = self.availability.status == AvailabilityStatus::Supported;
        require(
            if supported {
                self.availability.reason.is_none()
            } else {
                self.availability
                    .reason
                    .as_deref()
                    .is_some_and(|r| text(r, 1024))
            },
            "availability reason disagrees with status",
        )?;
        require(
            match self.default.source {
                DefaultSource::Unknown => self.default.value.is_none(),
                _ => self
                    .default
                    .value
                    .as_ref()
                    .is_some_and(|v| self.value_type.accepts(v)),
            },
            "default value disagrees with evidence or type",
        )?;
        require(
            !(self.adjustable && self.scope == ParameterScope::Voice)
                || self.default.reset_supported,
            "adjustable voice parameter lacks reset support",
        )?;
        require(
            self.side_effects.len() <= MAX_PARAMETERS,
            "too many side effects",
        )?;
        let mut seen = BTreeSet::new();
        for id in &self.side_effects {
            require(
                identifier(id) && id != &self.id && seen.insert(id),
                "invalid side effect",
            )?;
        }
        Ok(())
    }

    pub(super) fn error(&self, reason: &'static str) -> ParameterError {
        ParameterError::Parameter {
            id: self.id.clone(),
            reason,
        }
    }
}

impl ParameterCatalogue {
    pub fn from_json(json: &[u8]) -> Result<Self, ParameterError> {
        let result: Self = decode(json)?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), ParameterError> {
        require(identifier(&self.engine_id), "invalid engine ID")?;
        self.identity.validate()?;
        // Physical voice IDs retain their existing opaque grammar, including + and paths.
        require(
            self.voice_id
                .as_deref()
                .is_none_or(|v| text(v, crate::logical_voices::MAX_PHYSICAL_VOICE_ID_BYTES)),
            "invalid physical voice ID",
        )?;
        require(
            self.parameters.len() <= MAX_PARAMETERS,
            "too many descriptors",
        )?;
        let mut ids = BTreeSet::new();
        for p in &self.parameters {
            p.validate()?;
            require(ids.insert(&p.id), "duplicate parameter ID")?;
            require(
                self.voice_id.is_some() || p.default.source != DefaultSource::RuntimeReadback,
                "voice readback requires physical voice identity",
            )?;
        }
        for p in &self.parameters {
            require(
                p.side_effects.iter().all(|id| ids.contains(id)),
                "unknown side effect target",
            )?;
        }
        require(self.mappings.len() <= MAX_PARAMETERS, "too many mappings")?;
        for m in &self.mappings {
            require(
                !m.common_inputs.is_empty()
                    && m.common_inputs.len() <= 14
                    && m.common_inputs.iter().collect::<BTreeSet<_>>().len()
                        == m.common_inputs.len(),
                "invalid common inputs",
            )?;
            require(
                !m.native_outputs.is_empty()
                    && m.native_outputs.len() <= MAX_PARAMETERS
                    && m.native_outputs.iter().all(|id| ids.contains(id))
                    && m.native_outputs.iter().collect::<BTreeSet<_>>().len()
                        == m.native_outputs.len(),
                "invalid native mapping outputs",
            )?;
        }
        Ok(())
    }
}

impl NativePatch {
    /// Decode inert data without requiring an installed engine. Never executes it.
    pub fn from_json(json: &[u8]) -> Result<Self, ParameterError> {
        let result: Self = decode(json)?;
        result.validate_shape()?;
        Ok(result)
    }

    pub fn validate_shape(&self) -> Result<(), ParameterError> {
        require(
            identifier(&self.engine_id) && identifier(&self.schema_id),
            "invalid native identity",
        )?;
        require(
            self.parameters.len() <= MAX_NATIVE_OPERATIONS,
            "too many native operations",
        )?;
        for (id, operation) in &self.parameters {
            require(identifier(id), "invalid native parameter ID")?;
            if let Adjustment::Set { value } = operation {
                value.validate()?;
            }
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        catalogue: &ParameterCatalogue,
        selector: &VoiceSelector,
    ) -> Result<(), ParameterError> {
        self.validate_shape()?;
        catalogue.validate()?;
        if selector.engine_id() != Some(self.engine_id.as_str())
            || self.engine_id != catalogue.engine_id
            || self.schema_id != catalogue.identity.schema_id
        {
            return Err(ParameterError::IdentityMismatch);
        }
        if let VoiceSelector::Exact(selected) = selector {
            if catalogue
                .voice_id
                .as_ref()
                .is_some_and(|v| *v != selected.voice_id)
            {
                return Err(ParameterError::IdentityMismatch);
            }
        }
        for (id, operation) in &self.parameters {
            let p = catalogue
                .parameters
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| ParameterError::Parameter {
                    id: id.clone(),
                    reason: "not described by runtime",
                })?;
            if p.scope != ParameterScope::Voice
                || !p.adjustable
                || p.availability.status != AvailabilityStatus::Supported
            {
                return Err(p.error("not adjustable for this voice/runtime"));
            }
            if let Adjustment::Set { value } = operation {
                if !p.value_type.accepts(value) {
                    return Err(p.error("value outside declared type/range"));
                }
            }
        }
        Ok(())
    }
}
