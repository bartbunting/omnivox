//! Bundled variant discovery and explicit physical combinations. No downloads.

use super::*;

pub const MAX_CHOICES: usize = 64;
const MAX_VARIANTS: usize = 512;
const MAX_SETTINGS_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EspeakVariantChoice {
    pub base_voice_id: String,
    pub variant_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EspeakVariant {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct EspeakVariantCatalogue {
    pub schema_version: u32,
    pub bases: Vec<VoiceDescriptor>,
    pub variants: Vec<EspeakVariant>,
    pub max_choices: usize,
    pub max_native_identifier_bytes: usize,
}

fn invalid(message: impl Into<String>) -> TtsError {
    TtsError::InvalidParameter(message.into())
}

pub(super) fn configured_choices() -> Result<Vec<EspeakVariantChoice>, TtsError> {
    match std::env::var("OMNIVOX_ESPEAK_VARIANTS") {
        Ok(value) if value.is_empty() => Ok(Vec::new()),
        Ok(value) => parse_choices(&value),
        Err(std::env::VarError::NotPresent) => Ok(Vec::new()),
        Err(_) => Err(invalid("OMNIVOX_ESPEAK_VARIANTS must be UTF-8 JSON")),
    }
}

fn parse_choices(value: &str) -> Result<Vec<EspeakVariantChoice>, TtsError> {
    if value.len() > MAX_SETTINGS_BYTES {
        return Err(invalid("OMNIVOX_ESPEAK_VARIANTS exceeds 16 KiB"));
    }
    let choices: Vec<EspeakVariantChoice> = serde_json::from_str(value)
        .map_err(|error| invalid(format!("Invalid OMNIVOX_ESPEAK_VARIANTS: {error}")))?;
    validate_choices(&choices)?;
    Ok(choices)
}

fn combination(choice: &EspeakVariantChoice) -> Result<String, TtsError> {
    let base = choice
        .base_voice_id
        .strip_prefix("espeak:")
        .ok_or_else(|| invalid("eSpeak variant base must be an exact espeak: voice ID"))?;
    // The pinned native library appends the variant to a 40-byte identifier.
    // Reject numeric aliases, traversal, nested variants and truncation up front.
    let safe_part = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if !base.split(['/', '\\']).all(safe_part)
        || !safe_part(&choice.variant_id)
        || choice.variant_id.as_bytes()[0].is_ascii_digit()
        || base.len() + 1 + choice.variant_id.len() > 39
    {
        return Err(invalid(
            "Invalid or overlong eSpeak base/variant combination",
        ));
    }
    Ok(format!("{}+{}", choice.base_voice_id, choice.variant_id))
}

fn validate_choices(choices: &[EspeakVariantChoice]) -> Result<(), TtsError> {
    if choices.len() > MAX_CHOICES {
        return Err(invalid(
            "At most 64 eSpeak variant combinations may be configured",
        ));
    }
    let mut ids = HashSet::new();
    for choice in choices {
        if !ids.insert(combination(choice)?) {
            return Err(invalid("Repeated eSpeak variant combination"));
        }
    }
    Ok(())
}

impl EspeakTtsEngine {
    /// Discover from this process's native data, independent of the base cache.
    /// The native list is copied while locked; eSpeak owns and replaces its pointers.
    pub fn variant_catalogue(&self) -> Result<EspeakVariantCatalogue, TtsError> {
        let state = ESPEAK_LOCK.get().ok_or(TtsError::NotAvailable)?;
        let _guard = state
            .lock()
            .map_err(|_| invalid("eSpeak state lock poisoned"))?;
        let language = CString::new("!v").unwrap();
        let mut variants = Vec::new();
        unsafe {
            let mut spec: espeak_rs_sys::espeak_VOICE = std::mem::zeroed();
            spec.languages = language.as_ptr();
            let list = espeak_rs_sys::espeak_ListVoices(&mut spec);
            if !list.is_null() {
                for index in 0..=MAX_VARIANTS {
                    let entry = *list.add(index);
                    if entry.is_null() {
                        break;
                    }
                    if index == MAX_VARIANTS {
                        return Err(invalid("Too many native eSpeak variants"));
                    }
                    let entry = &*entry;
                    if entry.identifier.is_null() || entry.name.is_null() {
                        continue;
                    }
                    let identifier = CStr::from_ptr(entry.identifier).to_string_lossy();
                    let Some(id) = identifier
                        .strip_prefix("!v/")
                        .or_else(|| identifier.strip_prefix("!v\\"))
                    else {
                        continue;
                    };
                    // Validate using a short real-shaped base before exposing a native suffix.
                    if combination(&EspeakVariantChoice {
                        base_voice_id: "espeak:en".into(),
                        variant_id: id.into(),
                        enabled: true,
                    })
                    .is_err()
                    {
                        continue;
                    }
                    variants.push(EspeakVariant {
                        id: id.into(),
                        display_name: CStr::from_ptr(entry.name).to_string_lossy().into_owned(),
                    });
                }
            }
        }
        variants.sort_by(|a, b| a.id.cmp(&b.id));
        variants.dedup_by(|a, b| a.id == b.id);
        Ok(EspeakVariantCatalogue {
            schema_version: 1,
            bases: self
                .descriptor
                .voices
                .iter()
                .filter(|v| !v.id.voice_id.contains('+'))
                .cloned()
                .collect(),
            variants,
            max_choices: MAX_CHOICES,
            max_native_identifier_bytes: 39,
        })
    }

    pub(super) fn add_variant_choices(
        &mut self,
        choices: &[EspeakVariantChoice],
    ) -> Result<(), TtsError> {
        validate_choices(choices)?;
        if choices.is_empty() {
            return Ok(());
        }
        let catalogue = self.variant_catalogue()?;
        for choice in choices {
            if !choice.enabled {
                continue;
            }
            let id = combination(choice)?;
            let base = catalogue
                .bases
                .iter()
                .find(|v| v.id.voice_id == choice.base_voice_id);
            let variant = catalogue
                .variants
                .iter()
                .find(|v| v.id == choice.variant_id);
            let reason = if base.is_none() {
                Some("eSpeak base voice is missing from this speech host")
            } else if variant.is_none() {
                Some("eSpeak variant is missing from this speech host")
            } else {
                None
            };
            self.descriptor.voices.push(VoiceDescriptor {
                id: PhysicalVoiceId::new("espeak", id),
                display_name: format!(
                    "{} + {}",
                    base.map_or(choice.base_voice_id.as_str(), |v| v.display_name.as_str()),
                    variant.map_or(choice.variant_id.as_str(), |v| v.display_name.as_str())
                ),
                language: base.and_then(|v| v.language.clone()),
                gender: None,
                quality: VoiceQuality::Compact,
                availability: reason.map_or(Availability::Available, |reason| {
                    Availability::Unavailable {
                        reason: reason.into(),
                    }
                }),
            });
        }
        Ok(())
    }

    pub(super) fn check_variant_voice(&self, voice_id: &str) -> Result<(), TtsError> {
        if voice_id.contains('+')
            && !self
                .descriptor
                .voices
                .iter()
                .any(|v| v.id.voice_id == voice_id && v.availability == Availability::Available)
        {
            return Err(TtsError::VoiceNotFound(voice_id.into()));
        }
        Ok(())
    }

    /// Called with ESPEAK_LOCK held, before any PCM or start receipt is emitted.
    pub(super) fn select_native_voice(
        &self,
        voice_id: &str,
    ) -> Result<Option<PhysicalVoiceId>, TtsError> {
        self.check_variant_voice(voice_id)?;
        let variant = voice_id.contains('+');
        let name = CString::new(Self::backend_voice_name(voice_id))
            .map_err(|_| invalid("Invalid eSpeak voice name"))?;
        unsafe {
            if espeak_rs_sys::espeak_SetVoiceByName(name.as_ptr())
                != espeak_rs_sys::espeak_ERROR_EE_OK
            {
                return Err(TtsError::VoiceNotFound(voice_id.into()));
            }
            let current = espeak_rs_sys::espeak_GetCurrentVoice();
            let actual = if current.is_null() {
                None
            } else {
                let entry = &*current;
                let identifier = if entry.identifier.is_null() {
                    entry.name
                } else {
                    entry.identifier
                };
                (!identifier.is_null()).then(|| {
                    PhysicalVoiceId::new(
                        "espeak",
                        Self::reported_voice_id(
                            voice_id,
                            &CStr::from_ptr(identifier).to_string_lossy(),
                        ),
                    )
                })
            };
            // SetVoiceByName ignores a failed variant load. A base-only success
            // must never be reported as a successful exact variant audition.
            if variant && actual.as_ref().is_none_or(|id| id.voice_id != voice_id) {
                return Err(TtsError::VoiceNotFound(voice_id.into()));
            }
            Ok(actual)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Capture {
        voice: Option<PhysicalVoiceId>,
        samples: usize,
    }

    impl SynthesisStreamSink for Capture {
        fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
            self.voice = start.actual_voice;
            Ok(())
        }
        fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
            self.samples += audio.samples.len();
            Ok(())
        }
        fn markers(
            &mut self,
            _: Vec<SynthesisMarker>,
            _: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            Ok(())
        }
    }

    fn choice(base: &str, variant: &str, enabled: bool) -> EspeakVariantChoice {
        EspeakVariantChoice {
            base_voice_id: base.into(),
            variant_id: variant.into(),
            enabled,
        }
    }

    #[test]
    fn bounded_combinations_reject_native_aliases_traversal_and_overflow() {
        for (base, suffix) in [
            ("en", "m1"),
            ("espeak:../en", "m1"),
            ("espeak:en", "../m1"),
            ("espeak:en", "1"),
            ("espeak:en+m1", "f1"),
            ("espeak:en", ""),
        ] {
            assert!(combination(&choice(base, suffix, true)).is_err());
        }
        assert!(combination(&choice("espeak:gmw/en-US", &"a".repeat(35), true)).is_err());
        assert_eq!(
            combination(&choice("espeak:gmw\\en-US", "m1", true)).unwrap(),
            "espeak:gmw\\en-US+m1"
        );
        assert!(parse_choices(&" ".repeat(MAX_SETTINGS_BYTES + 1)).is_err());
        assert!(validate_choices(&[
            choice("espeak:en", "m1", true),
            choice("espeak:en", "m1", false)
        ])
        .is_err());
        assert!(parse_choices(r#"[{"base_voice_id":"espeak:en","variant_id":"m1"}]"#).is_err());
    }

    #[test]
    fn native_variants_are_explicit_exact_and_disabled_without_losing_base() {
        let plain = EspeakTtsEngine::with_variant_choices(&[]).unwrap();
        let catalogue = plain.variant_catalogue().unwrap();
        assert!(catalogue.variants.iter().any(|v| v.id == "m1"));
        let base = catalogue
            .bases
            .iter()
            .find(|v| v.language.as_deref() == Some("en-us"))
            .unwrap();
        let mut engine = EspeakTtsEngine::with_variant_choices(&[
            choice(&base.id.voice_id, "m1", true),
            choice(&base.id.voice_id, "f1", false),
            choice(&base.id.voice_id, "no-such-variant", true),
        ])
        .unwrap();
        assert_eq!(
            engine.descriptor.default_voice_id,
            plain.descriptor.default_voice_id
        );
        let id = format!("{}+m1", base.id.voice_id);
        let request =
            SynthesisRequest::new("Testing the selected variant.", TtsSettings::default())
                .with_route("variant-test", PhysicalVoiceId::new("espeak", &id));
        let result = engine.synthesize(&request).unwrap();
        assert_eq!(
            result.actual_voice,
            Some(PhysicalVoiceId::new("espeak", &id))
        );
        assert!(!result.audio.samples.is_empty());
        let mut stream = Capture::default();
        engine.synthesize_stream(&request, &mut stream).unwrap();
        assert_eq!(stream.voice, result.actual_voice);
        assert!(stream.samples > 0);
        let base_request = request.clone().with_route("base", base.id.clone());
        assert_eq!(
            engine.synthesize(&base_request).unwrap().actual_voice,
            Some(base.id.clone())
        );
        let mut faster = request.clone();
        faster.settings.rate = 0.7;
        faster.settings.pitch = 1.5;
        let fast_result = engine.synthesize(&faster).unwrap();
        assert_eq!(fast_result.actual_voice, result.actual_voice);
        assert!(fast_result.audio.samples.len() < result.audio.samples.len());
        for suffix in ["f1", "no-such-variant", "m2"] {
            let request = request.clone().with_route(
                "variant-test",
                PhysicalVoiceId::new("espeak", format!("{}+{suffix}", base.id.voice_id)),
            );
            assert!(matches!(
                engine.synthesize(&request),
                Err(TtsError::VoiceNotFound(_))
            ));
            let mut stream = Capture::default();
            assert!(engine.synthesize_stream(&request, &mut stream).is_err());
            assert_eq!(stream.samples, 0);
            assert_eq!(stream.voice, None);
        }
        // Even a stale native descriptor cannot turn a missing suffix into base speech.
        engine.descriptor.voices.last_mut().unwrap().availability = Availability::Available;
        let missing = request.clone().with_route(
            "variant-test",
            PhysicalVoiceId::new("espeak", format!("{}+no-such-variant", base.id.voice_id)),
        );
        assert!(matches!(
            engine.synthesize(&missing),
            Err(TtsError::VoiceNotFound(_))
        ));
        assert!(plain.synthesize(&request).is_err());
    }
}
