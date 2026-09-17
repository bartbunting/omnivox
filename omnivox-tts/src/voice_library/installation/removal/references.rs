use super::*;
use crate::voice_library::local::Startup;
use crate::voice_library::operations::Inspection;

/// The returned profile owners remain held through detachment and unlinking.
/// Every lock is nonblocking, including cross-profile references.
pub(super) fn check(
    profile: &Profile,
    removal: &Removal,
) -> Result<(Vec<Profile>, Vec<String>), LibraryError> {
    let root = profile.storage_root()?;
    let mut others = Vec::new();
    let mut reasons = Vec::new();
    for path in retention::entries(&root.join("profiles"), 64)? {
        ordinary(&path, false)?;
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LibraryError::Invalid("invalid library profile name"))?;
        uuid(id)?;
        if id != profile.index.document().profile_id {
            others.push(Profile::open(&root, id)?);
        }
    }
    for owner in std::iter::once(profile).chain(others.iter()) {
        for claim in owner.admission.inspect()? {
            require(
                matches!(
                    claim.state,
                    Inspection::Staged
                        | Inspection::Cancelled
                        | Inspection::Failed
                        | Inspection::Abandoned
                ),
                "unresolved native validation retains package files",
            )?;
        }
        let activations = owner.path.join("activations");
        if activations.try_exists()? {
            for path in retention::entries(&activations, 4096)? {
                ordinary(&path, false)?;
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Completion {
                    sequence: usize,
                    state: String,
                }
                let completed: Completion = decode(
                    &read_bounded(open_file(&path.join("completed.json"), false)?, 4096)?,
                    4096,
                )?;
                require(
                    completed.sequence < 64
                        && matches!(
                            completed.state.as_str(),
                            "succeeded" | "rolled-back" | "cancelled" | "failed"
                        ),
                    "unfinished Apply retains its candidate and rollback files",
                )?;
            }
        }
        for package in &owner.index.document().packages {
            if owner.index.document().profile_id == profile.index.document().profile_id
                && package.package_id == removal.package.package_id
                && package.revision_id == removal.package.revision_id
            {
                continue;
            }
            if package
                .files
                .iter()
                .any(|file| uses(&file.path, &removal.directory))
            {
                reasons.push(format!(
                    "Shared files remain installed in profile {}",
                    owner.index.document().profile_id
                ));
            }
        }
        if let Some(active) = owner.active_json()? {
            let pointer = ActivePointer::parse(active.as_bytes())?;
            let library = RuntimeLibrary::read(
                open_file(&owner.generation_path(&pointer.generation_id), false)?,
                host(),
            )?;
            if library_uses(&library, &removal.directory) {
                reasons.push(format!(
                    "Active generation {} still uses this package; disable its voices and Apply",
                    pointer.generation_id
                ));
            }
        }
    }
    for path in retention::entries(&root.join("sessions"), 16384)? {
        ordinary(&path, true)?;
        if path.extension().and_then(|name| name.to_str()) != Some("json") {
            continue;
        }
        uuid(
            path.file_stem()
                .and_then(|name| name.to_str())
                .ok_or(LibraryError::Invalid("invalid session snapshot name"))?,
        )?;
        let bytes = read_bounded(open_file(&path, false)?, MAX_RUNTIME_BYTES)?;
        if retention::released(&path, &bytes)? {
            continue;
        }
        let startup: Startup = decode(&bytes, MAX_RUNTIME_BYTES)?;
        let mut referenced = false;
        if let Some(generation) = startup.environment.get("OMNIVOX_VOICE_LIBRARY") {
            let library = RuntimeLibrary::read(open_file(Path::new(generation), false)?, host())?;
            require(
                startup.configuration.as_ref() == Some(&library.configuration()),
                "session generation identity changed",
            )?;
            referenced |= library_uses(&library, &removal.directory);
        } else {
            require(
                startup.configuration.is_none(),
                "session generation reference is missing",
            )?;
        }
        if let Some(model) = startup.environment.get("OMNIVOX_PIPER_MODEL") {
            referenced |= uses(model, &removal.directory);
        }
        if let Some(voices) = startup.environment.get("OMNIVOX_FLITE_VOICES") {
            referenced |= std::env::split_paths(voices)
                .any(|path| uses(&path.to_string_lossy(), &removal.directory));
        }
        for name in [
            "OMNIVOX_RHVOICE_DATA",
            "OMNIVOX_RHVOICE_RESOURCES",
            "RHVOICE_DATA_PATH",
        ] {
            if let Some(paths) = startup.environment.get(name) {
                referenced |= std::env::split_paths(paths)
                    .any(|p| uses(&p.to_string_lossy(), &removal.directory));
            }
        }
        if referenced {
            reasons.push(format!(
                "Session {} has no confirmed retirement; its files are retained",
                path.file_stem().unwrap().to_string_lossy()
            ));
        }
    }
    reasons.sort();
    reasons.dedup();
    require(
        reasons.len() <= 128,
        "too many retained references; cleanup refused",
    )?;
    Ok((others, reasons))
}

fn normalized(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let text = resolved.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.strip_prefix("//?/").unwrap_or(&text).to_lowercase()
    } else {
        text
    }
}
fn uses(path: &str, directory: &Path) -> bool {
    let path = normalized(Path::new(path));
    let directory = normalized(directory);
    path == directory || path.starts_with(&(directory + "/"))
}
fn library_uses(library: &RuntimeLibrary, directory: &Path) -> bool {
    let doc = library.document();
    doc.rhvoice.as_ref().is_some_and(|r| {
        r.voices
            .iter()
            .any(|v| v.files.iter().any(|f| uses(&f.path, directory)))
    }) || doc.piper.as_ref().is_some_and(|piper| {
        piper
            .models
            .iter()
            .any(|model| uses(&model.model.path, directory) || uses(&model.config.path, directory))
    }) || doc.flite.as_ref().is_some_and(|flite| {
        flite
            .files
            .iter()
            .any(|voice| uses(&voice.file.path, directory))
    }) || doc.mbrola.as_ref().is_some_and(|mbrola| {
        mbrola
            .files
            .iter()
            .any(|voice| uses(&voice.database.path, directory))
    })
}
