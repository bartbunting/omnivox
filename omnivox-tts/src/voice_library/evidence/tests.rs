use super::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    root: PathBuf,
    helper: PathBuf,
    validator: PathBuf,
    library: RuntimeLibrary,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "omnivox-evidence-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let directory = root.join("flite");
        fs::create_dir(&directory).unwrap();
        let helper = directory.join(format!(
            "omnivox-flite-helper{}",
            std::env::consts::EXE_SUFFIX
        ));
        let validator = root.join("validator");
        fs::write(&helper, b"helper bytes").unwrap();
        fs::write(&validator, b"validator bytes").unwrap();
        let target = if cfg!(target_os = "macos") {
            "apple-darwin"
        } else if cfg!(windows) && cfg!(target_env = "msvc") {
            "pc-windows-msvc"
        } else if cfg!(windows) {
            "pc-windows-gnu"
        } else if cfg!(target_env = "musl") {
            "unknown-linux-musl"
        } else {
            "unknown-linux-gnu"
        };
        fs::write(
            directory.join("SOURCE-PROVENANCE.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "artifact": "omnivox-flite-companion-test",
                "target": format!("{}-{target}", std::env::consts::ARCH)
            }))
            .unwrap(),
        )
        .unwrap();
        let library = RuntimeLibrary::parse(
            br#"{"schema_version":1,
            "target_id":"11111111-1111-4111-8111-111111111111",
            "profile_id":"22222222-2222-4222-8222-222222222222",
            "generation_id":"33333333-3333-4333-8333-333333333333",
            "piper":null,"flite":{"builtin_slt":true,"files":[]},"disabled_physical_ids":[]}"#,
            host(std::env::consts::OS).unwrap(),
        )
        .unwrap();
        let fixture = Self {
            root,
            helper,
            validator,
            library,
        };
        fixture.manifest();
        fixture
    }
    fn manifest(&self) {
        let root = self.helper.parent().unwrap();
        let mut lines = Vec::new();
        fn collect(root: &std::path::Path, directory: &std::path::Path, lines: &mut Vec<String>) {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(root, &path, lines);
                    continue;
                }
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                if name == "SHA256SUMS" {
                    continue;
                }
                lines.push(format!(
                    "{}  {name}\n",
                    companion::file_identity(&path).unwrap().sha256
                ));
            }
        }
        collect(root, root, &mut lines);
        lines.sort();
        fs::write(root.join("SHA256SUMS"), lines.concat()).unwrap();
    }
    fn capture(&self) -> Result<EvidenceSnapshot, LibraryError> {
        EvidenceSnapshot::capture(
            &self.library,
            &self.validator,
            &BTreeMap::from([("flite".into(), self.helper.clone())]),
            60,
            4096 * 1024 * 1024,
        )
    }
}

#[test]
fn piper_data_and_native_library_changes_remain_visible_after_rehashing() {
    let mut fixture = Fixture::new();
    let root = fixture.helper.parent().unwrap().to_path_buf();
    let helper = root.join(format!(
        "omnivox-piper-helper{}",
        std::env::consts::EXE_SUFFIX
    ));
    fs::rename(&fixture.helper, &helper).unwrap();
    fixture.helper = helper;
    let provenance = fs::read_to_string(root.join("SOURCE-PROVENANCE.json"))
        .unwrap()
        .replace("omnivox-flite-companion", "omnivox-piper-companion");
    fs::write(root.join("SOURCE-PROVENANCE.json"), provenance).unwrap();
    fs::create_dir(root.join("espeak-ng-data")).unwrap();
    fs::write(root.join("espeak-ng-data/phontab"), b"phonemes").unwrap();
    let libraries = if cfg!(windows) {
        ["piper.dll", "onnxruntime.dll"]
    } else if cfg!(target_os = "macos") {
        ["libpiper.dylib", "libonnxruntime.1.22.0.dylib"]
    } else {
        ["libpiper.so", "libonnxruntime.so.1"]
    };
    for name in libraries {
        fs::write(root.join(name), b"native library").unwrap();
    }
    fixture.manifest();
    let original = companion::capture(&fixture.helper, "piper").unwrap();
    for name in [
        "espeak-ng-data/phontab",
        libraries[0],
        libraries[1],
        "SOURCE-PROVENANCE.json",
    ] {
        let path = root.join(name);
        let previous = fs::read(&path).unwrap();
        let mut changed = previous.clone();
        changed.push(b' ');
        fs::write(&path, changed).unwrap();
        assert!(companion::capture(&fixture.helper, "piper").is_err());
        fixture.manifest();
        assert_ne!(
            original,
            companion::capture(&fixture.helper, "piper").unwrap()
        );
        fs::write(&path, previous).unwrap();
        fixture.manifest();
    }
    fs::remove_file(root.join("espeak-ng-data/phontab")).unwrap();
    fixture.manifest();
    assert!(companion::capture(&fixture.helper, "piper").is_err());
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn saved_evidence_detects_content_changes_even_with_updated_checksums() {
    let fixture = Fixture::new();
    let first = fixture.capture().unwrap();
    let report = ValidationEvidence::after_success(first.clone(), 123);
    assert!(
        ValidationEvidence::read(report.to_bytes().unwrap().as_slice())
            .unwrap()
            .matches(&first)
    );
    fs::write(&fixture.helper, b"changed bytes").unwrap();
    assert!(fixture.capture().is_err());
    fixture.manifest();
    assert!(!report.matches(&fixture.capture().unwrap()));
    fs::write(&fixture.helper, b"helper bytes").unwrap();
    fixture.manifest();
    assert!(report.matches(&fixture.capture().unwrap()));
    fs::write(&fixture.validator, b"replaced validator").unwrap();
    assert!(!report.matches(&fixture.capture().unwrap()));
    let mut changed = first.clone();
    changed.timeout_seconds += 1;
    assert!(!report.matches(&changed));
    changed = first.clone();
    changed.memory_bytes += 1024 * 1024;
    assert!(!report.matches(&changed));
    changed = first;
    changed.arch = "other-architecture".into();
    assert!(!report.matches(&changed));
}

#[test]
fn exact_generation_bytes_and_every_enabled_speaker_are_bound() {
    let mut fixture = Fixture::new();
    let original = ValidationEvidence::after_success(fixture.capture().unwrap(), 123);
    let bytes = format!(
        "{}\n",
        String::from_utf8(fixture.library.source_bytes().to_vec()).unwrap()
    );
    fixture.library =
        RuntimeLibrary::parse(bytes.as_bytes(), host(std::env::consts::OS).unwrap()).unwrap();
    assert!(!original.matches(&fixture.capture().unwrap()));

    let mut document = fixture.library.document().clone();
    document.piper = Some(super::super::PiperLibrary {
        models: vec![super::super::PiperModel {
            identity: super::super::ModelIdentity::Catalogue {
                catalogue_key: "fixture".into(),
            },
            model: super::super::AssetFile {
                path: if cfg!(windows) { "C:/model" } else { "/model" }.into(),
                bytes: 1,
                sha256: "a".repeat(64),
            },
            config: super::super::AssetFile {
                path: if cfg!(windows) {
                    "C:/config"
                } else {
                    "/config"
                }
                .into(),
                bytes: 1,
                sha256: "b".repeat(64),
            },
            voices: [0, 1]
                .into_iter()
                .map(|speaker| super::super::PiperVoice {
                    physical_id: format!("piper:v1/c/fixture/{speaker}"),
                    speaker_index: speaker,
                    display_name: format!("Speaker {speaker}"),
                    language: None,
                })
                .collect(),
        }],
    });
    let library = RuntimeLibrary::parse(
        &serde_json::to_vec(&document).unwrap(),
        host(std::env::consts::OS).unwrap(),
    )
    .unwrap();
    let units = loads(&library).unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].voices.len(), 2);
    assert_eq!(units[0].voices[1].voice_id, "piper:v1/c/fixture/1");
    assert_ne!(units[0].projection_sha256, library.sha256());
}

#[test]
fn inventory_rejects_missing_extra_duplicate_and_unsafe_files() {
    let fixture = Fixture::new();
    let root = fixture.helper.parent().unwrap();
    let path = root.join("SHA256SUMS");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(root.join("unlisted"), b"extra").unwrap();
    assert!(fixture.capture().is_err());
    fs::remove_file(root.join("unlisted")).unwrap();
    for contents in [
        format!("{original}{original}"),
        format!("{original}{}  ../escape\n", "a".repeat(64)),
        format!("{original}{}  missing\n", "a".repeat(64)),
        original.lines().next().unwrap().to_owned(),
        format!("{original}{}  SHA256SUMS\n", "a".repeat(64)),
    ] {
        fs::write(&path, contents).unwrap();
        assert!(fixture.capture().is_err());
    }
    for path in [
        "/absolute",
        "../escape",
        "a/../b",
        "a//b",
        "a\\b",
        "a:b",
        "NUL",
        "a.",
        "a ",
        "a\nb",
    ] {
        assert!(companion::relative_path(path).is_err(), "{path}");
    }
}

#[test]
fn companion_target_checks_preserve_executable_boundaries() {
    let fixture = Fixture::new();
    let path = fixture
        .helper
        .parent()
        .unwrap()
        .join("SOURCE-PROVENANCE.json");
    let mut provenance: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    #[cfg(windows)]
    {
        let other = if cfg!(target_env = "msvc") {
            "gnu"
        } else {
            "msvc"
        };
        provenance["target"] = format!("{}-pc-windows-{other}", std::env::consts::ARCH).into();
        fs::write(&path, serde_json::to_vec(&provenance).unwrap()).unwrap();
        fixture.manifest();
        fixture.capture().unwrap();
    }
    provenance["target"] = "unrelated-target".into();
    fs::write(&path, serde_json::to_vec(&provenance).unwrap()).unwrap();
    fixture.manifest();
    assert!(fixture.capture().is_err());
}

#[test]
fn saved_metadata_never_opens_its_paths_and_rejects_incomplete_reports() {
    let fixture = Fixture::new();
    let report = ValidationEvidence::after_success(fixture.capture().unwrap(), 123);
    let bytes = report.to_bytes().unwrap();
    fs::remove_file(&fixture.helper).unwrap();
    fs::remove_file(&fixture.validator).unwrap();
    assert!(ValidationEvidence::read(bytes.as_slice()).is_ok());
    assert!(ValidationEvidence::read(&bytes[..bytes.len() - 1]).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["cleanup_confirmed"] = false.into();
    assert!(ValidationEvidence::read(serde_json::to_vec(&value).unwrap().as_slice()).is_err());
    value["cleanup_confirmed"] = true.into();
    value["snapshot"]["loads"][0]["voices"] = serde_json::json!([]);
    assert!(ValidationEvidence::read(serde_json::to_vec(&value).unwrap().as_slice()).is_err());
    let text = String::from_utf8(bytes).unwrap();
    let duplicate = text.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    assert!(ValidationEvidence::read(duplicate.as_bytes()).is_err());
    assert!(
        ValidationEvidence::read(std::io::repeat(b' ').take(MAX_EVIDENCE_BYTES as u64 + 1))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn companion_symlinks_are_not_followed() {
    let fixture = Fixture::new();
    std::os::unix::fs::symlink(
        &fixture.validator,
        fixture.helper.parent().unwrap().join("linked"),
    )
    .unwrap();
    assert!(fixture.capture().is_err());
}
