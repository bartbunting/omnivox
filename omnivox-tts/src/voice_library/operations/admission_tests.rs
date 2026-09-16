use super::*;
#[path = "recovery_tests.rs"]
mod recovery_tests;
const TARGET: &str = "11111111-1111-4111-8111-111111111111";
const PROFILE: &str = "22222222-2222-4222-8222-222222222222";
const SECOND: &str = "55555555-5555-4555-8555-555555555555";

fn setup() -> Fixture {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("operations")).unwrap();
    fs::create_dir_all(fixture.0.join("profiles").join(PROFILE)).unwrap();
    Admission::create(&fixture.0, TARGET, PROFILE).unwrap();
    for id in [OPERATION_ID, SECOND] {
        let mut doc = plan().document().clone();
        doc.operation_id = id.into();
        Operation::create(
            &fixture.0.join("operations"),
            ValidationPlan::parse(&serde_json::to_vec(&doc).unwrap()).unwrap(),
        )
        .unwrap();
    }
    fixture
}
fn gate(root: &Path) -> Admission {
    Admission::try_open(root, PROFILE).unwrap().unwrap()
}
fn claim(root: &Path) -> PathBuf {
    root.join("profiles")
        .join(PROFILE)
        .join("validation-admission/claims")
        .join(OPERATION_ID)
}

#[test]
fn a_fresh_operation_id_cannot_bypass_unconfirmed_profile_work() {
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    assert!(Admission::try_open(&fixture.0, PROFILE).unwrap().is_none());
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    admitted.append(validating()).unwrap();
    drop(admitted);
    drop(owner);
    let mut owner = gate(&fixture.0);
    assert!(owner.admit(SECOND).is_err());
    assert!(owner.admit(OPERATION_ID).is_err());
    assert_eq!(owner.inspect().unwrap()[0].state, Inspection::Interrupted);
}

#[test]
fn only_confirmed_completion_allows_the_next_operation() {
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    admitted.append(validating()).unwrap();
    admitted.append(failed(Cleanup::Confirmed)).unwrap();
    drop(admitted);
    assert!(owner.admit(SECOND).is_ok());
    assert!(owner.admit(OPERATION_ID).is_err());
}

#[test]
fn prepared_claims_resume_only_the_same_operation() {
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    drop(owner.admit(OPERATION_ID).unwrap());
    assert!(owner.admit(SECOND).is_err());
    let original = fs::read(claim(&fixture.0)).unwrap();
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    admitted.append(failed(Cleanup::NotStarted)).unwrap();
    drop(admitted);
    assert_eq!(fs::read(claim(&fixture.0)).unwrap(), original);
    assert!(owner.admit(SECOND).is_ok());
}

#[test]
fn target_mismatch_and_incomplete_history_never_admit_a_worker() {
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    let path = fixture
        .0
        .join("operations")
        .join(OPERATION_ID)
        .join("plan.json");
    let original = fs::read(&path).unwrap();
    let changed = String::from_utf8(original.clone())
        .unwrap()
        .replace(TARGET, SECOND);
    fs::write(&path, changed).unwrap();
    assert!(owner.admit(OPERATION_ID).is_err());
    assert!(!claim(&fixture.0).exists());
    fs::write(path, original).unwrap();
    drop(owner.admit(OPERATION_ID).unwrap());
    let original = fs::read(claim(&fixture.0)).unwrap();
    fs::write(claim(&fixture.0), &original[..original.len() - 1]).unwrap();
    assert!(owner.inspect().is_err());
    assert!(owner.admit(SECOND).is_err());
    assert_eq!(
        fs::read(claim(&fixture.0)).unwrap(),
        &original[..original.len() - 1]
    );
}

#[test]
fn inspection_preserves_missing_initialization_and_missing_claimed_operations() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("operations")).unwrap();
    fs::create_dir_all(fixture.0.join("profiles").join(PROFILE)).unwrap();
    assert!(Admission::try_open(&fixture.0, PROFILE).is_err());
    assert_eq!(
        fs::read_dir(fixture.0.join("profiles").join(PROFILE))
            .unwrap()
            .count(),
        0
    );
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    drop(owner.admit(OPERATION_ID).unwrap());
    fs::rename(
        fixture.0.join("operations").join(OPERATION_ID),
        fixture.0.join("retained-operation"),
    )
    .unwrap();
    assert!(owner.inspect().is_err());
    assert!(owner.admit(SECOND).is_err());
    assert!(fixture.0.join("retained-operation").exists());
}

#[test]
fn native_owner_death_keeps_the_profile_claim() {
    for mode in [
        "claimed",
        "validating",
        "failed",
        "partial",
        "cleaned",
        "idle-workers",
    ] {
        let fixture = setup();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "voice_library::operations::tests::admission_tests::profile_owner_fixture",
                "--nocapture",
            ])
            .env("OMNIVOX_ADMISSION_FIXTURE_ROOT", &fixture.0)
            .env("OMNIVOX_ADMISSION_FIXTURE_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = OwnerChild(Some(child));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !fixture.0.join("ready").exists() {
            assert!(child.child().try_wait().unwrap().is_none());
            assert!(
                Instant::now() < deadline,
                "profile owner did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(Admission::try_open(&fixture.0, PROFILE).unwrap().is_none());
        child.kill_and_reap();
        let mut owner = gate(&fixture.0);
        assert_eq!(owner.admit(SECOND).is_ok(), mode == "failed", "{mode}");
        if mode == "claimed" {
            assert!(owner.admit(OPERATION_ID).is_ok());
        } else {
            assert!(owner.admit(OPERATION_ID).is_err());
        }
        if matches!(mode, "cleaned" | "idle-workers") {
            owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
            assert!(owner.admit(SECOND).is_ok());
        } else if mode != "claimed" {
            assert!(owner.abandon_cleaned_validation(OPERATION_ID).is_err());
        }
    }
}

#[test]
fn profile_owner_fixture() {
    let Some(root) = std::env::var_os("OMNIVOX_ADMISSION_FIXTURE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("OMNIVOX_ADMISSION_FIXTURE_MODE").unwrap();
    let mut owner = gate(&root);
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    if mode != "claimed" {
        admitted.append(validating()).unwrap();
    }
    if mode == "failed" {
        admitted.append(failed(Cleanup::Confirmed)).unwrap();
    }
    if mode == "partial" {
        let mut file = OpenOptions::new().append(true).open(claim(&root)).unwrap();
        file.write_all(b"partial").unwrap();
        file.sync_all().unwrap();
    }
    if matches!(mode.as_str(), "cleaned" | "idle-workers") {
        let mut records = ExecutionRecords::create(&admitted).unwrap();
        if mode == "cleaned" {
            records.starting("fixture-validator", &[]).unwrap();
            records.owned(std::process::id()).unwrap();
            records.cleaned().unwrap();
        }
    }
    fs::write(root.join("ready"), b"ready").unwrap();
    let _ = std::io::stdin().read_exact(&mut [0]);
}

#[test]
fn incomplete_worker_records_cannot_be_reused_or_completed() {
    let fixture = setup();
    let mut owner = gate(&fixture.0);
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    admitted.append(validating()).unwrap();
    let mut records = ExecutionRecords::create(&admitted).unwrap();
    assert!(ExecutionRecords::create(&admitted).is_err());
    records
        .starting("/fixture/validator", &["--internal-worker".into()])
        .unwrap();
    assert!(records.pending());
    assert!(records.starting("/fixture/validator", &[]).is_err());
    records.owned(std::process::id()).unwrap();
    let path = admitted
        .operation()
        .path()
        .join("workers/0000-cleaned.json");
    fs::write(&path, b"incomplete old record").unwrap();
    assert!(records.cleaned().is_err());
    assert!(records.pending());
    assert!(records.bind(admitted.operation().plan(), b"{}").is_err());
    assert_eq!(fs::read(path).unwrap(), b"incomplete old record");
    drop(admitted);
    assert!(owner.admit(SECOND).is_err());
}

#[test]
fn completion_evidence_is_bound_to_the_attempt_and_unchanged_worker_records() {
    use crate::voice_library::evidence::{EvidenceSnapshot, ValidationEvidence};
    let fixture = setup();
    let mut doc = plan().document().clone();
    doc.operation_id = "66666666-6666-4666-8666-666666666666".into();
    let mut generation: serde_json::Value = serde_json::from_str(&doc.generation_json).unwrap();
    generation["flite"] = serde_json::Value::Null;
    doc.generation_json = generation.to_string();
    doc.helpers.clear();
    let validator = fixture.0.join("validator");
    fs::write(&validator, b"fixture validator bytes").unwrap();
    doc.validator_path = validator.to_str().unwrap().into();
    let plan = ValidationPlan::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    Operation::create(&fixture.0.join("operations"), plan.clone()).unwrap();
    let mut owner = gate(&fixture.0);
    let mut admitted = owner.admit(&doc.operation_id).unwrap();
    admitted.append(validating()).unwrap();
    let mut records = ExecutionRecords::create(&admitted).unwrap();
    // Empty native load set still requires before/after observation workers.
    for _ in 0..2 {
        records
            .starting(doc.validator_path.as_str(), &["snapshot".into()])
            .unwrap();
        records.owned(std::process::id()).unwrap();
        records.cleaned().unwrap();
    }
    let snapshot = EvidenceSnapshot::capture(
        plan.generation(),
        &validator,
        &BTreeMap::new(),
        60,
        4096 * 1024 * 1024,
    )
    .unwrap();
    let report = ValidationEvidence::after_success(snapshot, 1)
        .to_bytes()
        .unwrap();
    let bound = records.bind(&plan, &report).unwrap();
    bound.save(&admitted).unwrap();
    assert!(bound.save(&admitted).is_err());
    let bytes = fs::read(admitted.operation().path().join("validation-evidence.json")).unwrap();
    BoundValidationEvidence::read(bytes.as_slice(), &plan).unwrap();
    doc.operation_id = SECOND.into();
    let other = ValidationPlan::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(BoundValidationEvidence::read(bytes.as_slice(), &other).is_err());
    fs::write(
        admitted.operation().path().join("workers/0000-owned.json"),
        b"changed",
    )
    .unwrap();
    assert!(records.bind(&plan, &report).is_err());
}
