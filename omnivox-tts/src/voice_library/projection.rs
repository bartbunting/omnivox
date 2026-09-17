//! Deterministic desired-state projection for explicit activation preparation.
use super::*;
use std::collections::BTreeMap;

impl LibraryIndex {
    /// Project only selected providers. An unselected provider retains legacy
    /// startup; a selected provider with no enabled voices has an empty managed
    /// load set. RHVoice additionally retains separately installed voices.
    /// This checks recorded validation identity, not current native readiness.
    pub fn project(
        &self,
        generation_id: &str,
        piper: bool,
        flite: bool,
        mbrola: bool,
        rhvoice: bool,
        host: HostPlatform,
    ) -> Result<RuntimeLibrary, LibraryError> {
        uuid(generation_id)?;
        let index = self.document();
        let mut models: BTreeMap<(&str, &str), (&PackageRevision, Vec<PiperVoice>)> =
            BTreeMap::new();
        let mut files = Vec::new();
        let mut builtin_slt = false;
        let mut builtin_en1 = false;
        let mut databases = Vec::new();
        let mut resources = Vec::new();
        for voice in index.voices.iter().filter(|voice| voice.enabled) {
            if !((piper && voice.engine_id == "piper")
                || (flite && voice.engine_id == "flite")
                || (mbrola && voice.engine_id == "mbrola")
                || (rhvoice && voice.engine_id == "rhvoice" && voice.package_id.is_some()))
            {
                continue;
            }
            if voice.engine_id == "flite" && voice.physical_id == "cmu_us_slt" {
                builtin_slt = true;
                continue;
            }
            if voice.engine_id == "mbrola"
                && voice.physical_id == MBROLA_EN1
                && voice.package_id.is_none()
            {
                builtin_en1 = true;
                continue;
            }
            let package = index
                .packages
                .iter()
                .find(|package| {
                    Some(&package.package_id) == voice.package_id.as_ref()
                        && Some(&package.revision_id) == voice.revision_id.as_ref()
                })
                .ok_or(LibraryError::Invalid("enabled voice has no package"))?;
            let validation = package.validation.as_ref().ok_or(LibraryError::Invalid(
                "enabled package has no native validation",
            ))?;
            require(
                validation.target_id == index.target_id
                    && validation.file_set_sha256 == package.file_set_sha256()?,
                "enabled package validation is stale or for another target",
            )?;
            if voice.engine_id == "piper" {
                let entry = models
                    .entry(package.identity.sort_key())
                    .or_insert_with(|| (package, Vec::new()));
                require(
                    entry.0.package_id == package.package_id
                        && entry.0.revision_id == package.revision_id,
                    "enabled speakers select different revisions of one model",
                )?;
                entry.1.push(PiperVoice {
                    physical_id: voice.physical_id.clone(),
                    speaker_index: voice.speaker_index.unwrap(),
                    display_name: voice.display_name.clone(),
                    language: voice.language.clone(),
                });
            } else if voice.engine_id == "rhvoice" {
                resources.push(RhvoiceVoice {
                    physical_id: voice.physical_id.clone(),
                    display_name: voice.display_name.clone(),
                    language: voice.language.clone(),
                    files: package
                        .files
                        .iter()
                        .map(|file| AssetFile {
                            path: file.path.clone(),
                            bytes: file.bytes,
                            sha256: file.sha256.clone(),
                        })
                        .collect(),
                });
            } else if voice.engine_id == "mbrola" {
                databases.push(MbrolaVoice {
                    physical_id: voice.physical_id.clone(),
                    database: asset(package, FileRole::Database)?,
                    display_name: voice.display_name.clone(),
                    language: voice.language.clone(),
                });
            } else {
                files.push(FliteVoice {
                    physical_id: voice.physical_id.clone(),
                    file: asset(package, FileRole::Voice)?,
                    display_name: voice.display_name.clone(),
                    language: voice.language.clone(),
                });
            }
        }
        let mut projected = Vec::new();
        for (_, (package, mut voices)) in models {
            voices.sort_by_key(|voice| voice.speaker_index);
            projected.push(PiperModel {
                identity: package.identity.clone(),
                model: asset(package, FileRole::Model)?,
                config: asset(package, FileRole::Config)?,
                voices,
            });
        }
        files.sort_by(|a, b| a.physical_id.cmp(&b.physical_id));
        databases.sort_by(|a, b| a.physical_id.cmp(&b.physical_id));
        resources.sort_by(|a, b| a.physical_id.cmp(&b.physical_id));
        let document = RuntimeDocument {
            schema_version: if rhvoice {
                3
            } else if mbrola {
                2
            } else {
                1
            },
            target_id: index.target_id.clone(),
            profile_id: index.profile_id.clone(),
            generation_id: generation_id.into(),
            disabled_physical_ids: index.disabled_physical_ids.clone(),
            piper: piper.then_some(PiperLibrary { models: projected }),
            flite: flite.then_some(FliteLibrary { builtin_slt, files }),
            rhvoice: rhvoice.then_some(RhvoiceLibrary {
                inherit_external: true,
                voices: resources,
            }),
            mbrola: mbrola.then_some(MbrolaLibrary {
                builtin_en1,
                files: databases,
            }),
        };
        RuntimeLibrary::parse(&serde_json::to_vec(&document)?, host)
    }
}

fn asset(package: &PackageRevision, role: FileRole) -> Result<AssetFile, LibraryError> {
    let file = package
        .files
        .iter()
        .find(|file| file.role == role)
        .ok_or(LibraryError::Invalid("missing projected asset role"))?;
    Ok(AssetFile {
        path: file.path.clone(),
        bytes: file.bytes,
        sha256: file.sha256.clone(),
    })
}

#[cfg(test)]
mod tests;
