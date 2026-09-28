use super::*;
use serde_json::json;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omnivox-punctuation-editor-{}",
            crate::voice_library::local::new_uuid().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> ConfigurationRoot {
        ConfigurationRoot {
            path: self.0.clone(),
            explicit: true,
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn review_is_read_only_and_save_preserves_unrelated_settings_and_permissions() {
    let dir = Directory::new();
    let path = dir.0.join("config.json");
    let original = json!({"schema":2,"speech":{"defaults":{"rate":0.7}},"audio":{"target":"left"},"routing":{"disabled_engine_ids":["piper"]}});
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
    let review = inspect(&dir.root()).unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    assert_eq!(
        review.effective.all.get(&'’').unwrap().as_deref(),
        Some("apostrophe")
    );
    let overrides = r#"{"some":{"’":"curly quote","$":null}}"#.as_bytes();
    let saved = save(&dir.root(), &review.sha256, overrides).unwrap();
    assert_ne!(saved.sha256, review.sha256);
    assert_eq!(
        saved.overrides,
        json!({"some":{"’":"curly quote","$":null}})
    );
    let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["schema"], 3);
    assert_eq!(
        document["speech"]["defaults"],
        original["speech"]["defaults"]
    );
    assert_eq!(document["audio"], original["audio"]);
    assert_eq!(document["routing"], original["routing"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    assert!(save(&dir.root(), &review.sha256, b"{}").is_err());
    assert_eq!(inspect(&dir.root()).unwrap().sha256, saved.sha256);
    let restored = save(&dir.root(), &saved.sha256, b"{}").unwrap();
    assert_eq!(restored.effective, PunctuationTables::default());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2); // config and permanent lock
}

#[test]
fn invalid_drafts_conflicts_and_changed_targets_never_overwrite() {
    let dir = Directory::new();
    let before = inspect(&dir.root()).unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    for invalid in [
        b"null".as_slice(),
        b"{\"some\":{\"!\":\"\"}}",
        b"{\"some\":{},\"some\":{}}",
    ] {
        assert!(save(&dir.root(), &before.sha256, invalid).is_err());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    }
    let other = Directory::new();
    assert!(save(&other.root(), &before.sha256, b"{}").is_err());
    let manual = b"{\"schema\":1}\n";
    fs::write(dir.0.join("config.json"), manual).unwrap();
    assert!(save(&dir.root(), &before.sha256, b"{}").is_err());
    assert_eq!(fs::read(dir.0.join("config.json")).unwrap(), manual);
    let current = inspect(&dir.root()).unwrap();
    let lease = options()
        .create(true)
        .truncate(false)
        .open(dir.0.join(".punctuation.lock"))
        .unwrap();
    lease.try_lock().unwrap();
    assert!(save(&dir.root(), &current.sha256, b"{}").is_err());
    assert_eq!(fs::read(dir.0.join("config.json")).unwrap(), manual);
    lease.unlock().unwrap();
}

#[test]
fn missing_default_root_is_created_only_on_save() {
    let dir = Directory::new();
    let root = ConfigurationRoot {
        path: dir.0.join("missing/omnivox"),
        explicit: false,
    };
    let before = inspect(&root).unwrap();
    assert!(!root.path.exists());
    let saved = save(&root, &before.sha256, b"{}").unwrap();
    assert!(root.path.join("config.json").is_file());
    assert_eq!(inspect(&root).unwrap().sha256, saved.sha256);
}

#[cfg(unix)]
#[test]
fn redirected_config_is_never_replaced() {
    let dir = Directory::new();
    let target = dir.0.join("other.json");
    fs::write(&target, b"{\"schema\":1}").unwrap();
    std::os::unix::fs::symlink(&target, dir.0.join("config.json")).unwrap();
    assert!(inspect(&dir.root()).is_err());
    assert!(save(&dir.root(), "", b"{}").is_err());
    assert!(fs::symlink_metadata(dir.0.join("config.json"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(target).unwrap(), b"{\"schema\":1}");
}
