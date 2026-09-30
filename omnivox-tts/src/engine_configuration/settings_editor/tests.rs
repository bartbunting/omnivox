use super::*;
use serde_json::json;
use std::path::PathBuf;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omnivox-engine-editor-{}",
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
fn review_is_read_only_and_save_retains_other_preferences() {
    let dir = Directory::new();
    let path = dir.0.join("config.json");
    let original = json!({"schema":3,"speech":{"defaults":{"rate":0.7},"punctuation":{"some":{"’":"quote"}}},"audio":{"target":"left"}});
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let review = inspect(&dir.root()).unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    let draft = json!({"routing":{"preferred_engine_ids":["espeak"],"fallback_engine_ids":[]},"engine_overrides":{"flite":{"enabled":false}}});
    let saved = save(
        &dir.root(),
        &review.sha256,
        &serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.settings, draft);
    let result: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(result["speech"], original["speech"]);
    assert_eq!(result["audio"], original["audio"]);
    assert!(save(
        &dir.root(),
        &review.sha256,
        &serde_json::to_vec(&draft).unwrap()
    )
    .is_err());
}

#[test]
fn punctuation_and_engine_edits_conflict_without_losing_either_draft() {
    let dir = Directory::new();
    let engine = inspect(&dir.root()).unwrap();
    let punctuation = punctuation_editor::inspect(&dir.root()).unwrap();
    punctuation_editor::save(&dir.root(), &punctuation.sha256, br#"{"all":{"!":"wow"}}"#).unwrap();
    let draft = br#"{"routing":{},"engine_overrides":{"espeak":{"enabled":true}}}"#;
    assert!(save(&dir.root(), &engine.sha256, draft).is_err());
    let latest = inspect(&dir.root()).unwrap();
    save(&dir.root(), &latest.sha256, draft).unwrap();
    assert_eq!(
        punctuation_editor::inspect(&dir.root())
            .unwrap()
            .effective
            .all
            .get(&'!')
            .unwrap()
            .as_deref(),
        Some("wow")
    );
}

#[test]
fn invalid_launch_overrides_do_not_write_configuration() {
    let dir = Directory::new();
    let review = inspect(&dir.root()).unwrap();
    for draft in [
        json!({"routing":{},"engine_overrides":{"espeak":{"arguments":["bad"]}}}),
        json!({"routing":{},"engine_overrides":{"unknown":{"enabled":true}}}),
        json!({"routing":{},"engine_overrides":{},"speech":{}}),
        json!({"routing":{"preferred_engine_ids":["espeak","espeak"]},"engine_overrides":{}}),
    ] {
        assert!(save(
            &dir.root(),
            &review.sha256,
            &serde_json::to_vec(&draft).unwrap()
        )
        .is_err());
        assert!(!dir.0.join("config.json").exists());
    }
}

#[test]
fn adding_an_installed_helper_is_disabled_and_does_not_replace_files() {
    let dir = Directory::new();
    let program = dir.0.join("helper");
    fs::write(&program, "not executed by the editor").unwrap();
    let review = inspect(&dir.root()).unwrap();
    let manifest =
        json!({"schema":1,"engine_id":"org.example.test","program":program,"enabled":false});
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let added = add_helper(&dir.root(), &review.sha256, &bytes).unwrap();
    assert!(added
        .engines
        .iter()
        .any(|engine| engine.engine_id == "org.example.test" && !engine.enabled));
    assert!(!dir.0.join("config.json").exists());
    assert!(add_helper(&dir.root(), &added.sha256, &bytes).is_err());
    let mut draft = added.settings;
    draft["engine_overrides"]["org.example.test"] = json!({"enabled":true});
    save(
        &dir.root(),
        &added.sha256,
        &serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::read(dir.0.join("helpers.d/org.example.test.json")).unwrap(),
        bytes
    );
}

#[test]
fn registration_rejects_implicit_activation_and_missing_programs() {
    let dir = Directory::new();
    let review = inspect(&dir.root()).unwrap();
    for enabled in [true, false] {
        let manifest = json!({"schema":1,"engine_id":"org.example.test","program":dir.0.join("missing"),"enabled":enabled});
        assert!(add_helper(
            &dir.root(),
            &review.sha256,
            &serde_json::to_vec(&manifest).unwrap()
        )
        .is_err());
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn concurrent_distinct_registrations_cannot_exceed_the_directory_limit() {
    let dir = Directory::new();
    let program = dir.0.join("helper");
    fs::write(&program, "not executed").unwrap();
    let helpers = dir.0.join("helpers.d");
    fs::create_dir(&helpers).unwrap();
    for index in 0..MAX_MANIFESTS - 1 {
        fs::write(helpers.join(format!("p{index}.json")), serde_json::to_vec(&json!({
            "schema":1, "engine_id":format!("org.fixture.p{index}"), "program":program, "enabled":false
        })).unwrap()).unwrap();
    }
    let root = dir.root();
    let revision = inspect(&root).unwrap().sha256;
    let barrier = std::sync::Barrier::new(2);
    let successes = std::thread::scope(|scope| {
        let handles = (0..2).map(|index| {
            let (root, revision, barrier, program) = (&root, &revision, &barrier, &program);
            scope.spawn(move || {
                barrier.wait();
                add_helper(root, revision, &serde_json::to_vec(&json!({
                    "schema":1, "engine_id":format!("org.fixture.new{index}"), "program":program, "enabled":false
                })).unwrap()).is_ok()
            })
        }).collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(successes, 1);
    let loaded = root.load().unwrap();
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(loaded.external.len(), MAX_MANIFESTS);
}
