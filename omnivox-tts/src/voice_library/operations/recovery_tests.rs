use super::*;

fn interrupted(root: &Path, workers: usize) -> PathBuf {
    let mut owner = gate(root);
    let mut admitted = owner.admit(OPERATION_ID).unwrap();
    admitted.append(validating()).unwrap();
    let mut records = ExecutionRecords::create(&admitted).unwrap();
    for _ in 0..workers {
        records.starting("unused-fixture-validator", &[]).unwrap();
        // Intentionally name this still-live test process. Recovery must only
        // read this diagnostic value, even when it is a valid current PID.
        records.owned(std::process::id()).unwrap();
        records.cleaned().unwrap();
    }
    admitted.operation().path().to_path_buf()
}

fn files(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            result.extend(files(&entry.path()));
        } else {
            // Match the canonical operation path, including Windows' extended
            // path prefix, when comparing snapshots with individual receipts.
            result.insert(
                entry.path().canonicalize().unwrap(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
    result
}

#[test]
fn recorded_cleanup_releases_only_new_work_and_preserves_original_evidence() {
    for workers in [0, 1, 3] {
        let fixture = setup();
        let operation = interrupted(&fixture.0, workers);
        // A partial report is retained; it is neither needed nor promoted.
        fs::write(
            operation.join("validation-evidence.json"),
            b"partial report",
        )
        .unwrap();
        let original = files(&fixture.0);
        let mut owner = gate(&fixture.0);
        assert!(owner.admit(SECOND).is_err());
        owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
        let receipt = operation.join("cleanup-recovery.frames");
        let saved = fs::read(&receipt).unwrap();
        // Windows locks exclude ordinary reads too. Snapshot all files only
        // after the profile owner has released its permanent lock file.
        drop(owner);
        let mut after = files(&fixture.0);
        after.remove(&receipt.canonicalize().unwrap()).unwrap();
        assert_eq!(after, original);
        let mut owner = gate(&fixture.0);
        owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
        assert_eq!(fs::read(receipt).unwrap(), saved);
        assert_eq!(
            Operation::inspect(&operation).unwrap(),
            Inspection::Abandoned
        );
        let mut reopened = Operation::try_open(&operation).unwrap().unwrap();
        assert_eq!(
            reopened.journal().state(),
            Some(ValidationState::Validating)
        );
        assert!(reopened.append(failed(Cleanup::Confirmed)).is_err());
        drop(reopened);
        assert!(owner.admit(OPERATION_ID).is_err());
        assert!(owner.admit(SECOND).is_ok());
    }
}

#[test]
fn incomplete_or_inconsistent_cleanup_never_changes_history_or_releases_admission() {
    for mode in [
        "missing-workers",
        "missing-intent",
        "missing-owner",
        "missing-cleanup",
        "extra-intent",
        "unknown-file",
        "gap",
        "torn-cleanup",
        "wrong-operation",
        "wrong-plan",
        "wrong-phase",
        "wrong-owner",
        "wrong-authority",
        "missing-null",
        "duplicate-key",
        "unknown-field",
        "zero-pid",
        "torn-journal",
        "too-many-workers",
    ] {
        let fixture = setup();
        let operation = interrupted(&fixture.0, if mode == "too-many-workers" { 4 } else { 1 });
        let workers = operation.join("workers");
        let cleaned = workers.join("0000-cleaned.json");
        match mode {
            "missing-workers" => fs::rename(&workers, operation.join("retained-workers")).unwrap(),
            "missing-intent" => fs::remove_file(workers.join("0000-intent.json")).unwrap(),
            "missing-owner" => fs::remove_file(workers.join("0000-owned.json")).unwrap(),
            "missing-cleanup" => fs::remove_file(&cleaned).unwrap(),
            "extra-intent" => fs::write(workers.join("0001-intent.json"), b"{}").unwrap(),
            "unknown-file" => fs::write(workers.join("unexpected"), b"{}").unwrap(),
            "gap" => fs::rename(&cleaned, workers.join("0001-cleaned.json")).unwrap(),
            "torn-cleanup" => {
                let bytes = fs::read(&cleaned).unwrap();
                fs::write(&cleaned, &bytes[..bytes.len() - 1]).unwrap();
            }
            "torn-journal" => {
                // Damage the validating frame itself: cleanup records must not
                // substitute for a verified admission-to-validation transition.
                let path = operation.join("journal.frames");
                let bytes = fs::read(&path).unwrap();
                fs::write(path, &bytes[..bytes.len() - 1]).unwrap();
            }
            "duplicate-key" => {
                let bytes = fs::read_to_string(&cleaned).unwrap();
                fs::write(&cleaned, bytes.replacen('{', "{\"schema_version\":1,", 1)).unwrap();
            }
            "too-many-workers" => (),
            _ => {
                let path = if mode == "zero-pid" {
                    workers.join("0000-owned.json")
                } else {
                    cleaned
                };
                let mut event: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                match mode {
                    "wrong-operation" => event["operation_id"] = SECOND.into(),
                    "wrong-plan" => event["plan_sha256"] = "a".repeat(64).into(),
                    "wrong-phase" => event["phase"] = "owned".into(),
                    "wrong-owner" => event["ownership"] = "unknown".into(),
                    "wrong-authority" => event["recovery_authority"] = "saved-pid".into(),
                    "unknown-field" => event["extra"] = true.into(),
                    "zero-pid" => event["pid"] = 0.into(),
                    "missing-null" => {
                        event.as_object_mut().unwrap().remove("pid");
                    }
                    _ => unreachable!(),
                }
                fs::write(path, event.to_string()).unwrap();
            }
        }
        let original = files(&fixture.0);
        let mut owner = gate(&fixture.0);
        assert!(
            owner.abandon_cleaned_validation(OPERATION_ID).is_err(),
            "{mode}"
        );
        assert!(owner.admit(SECOND).is_err(), "{mode}");
        drop(owner);
        assert_eq!(files(&fixture.0), original, "{mode}");
    }
}

#[test]
fn torn_completion_can_be_abandoned_without_repairing_or_reusing_the_attempt() {
    for cut in [1, 20, usize::MAX] {
        let fixture = setup();
        let operation = interrupted(&fixture.0, 1);
        let path = operation.join("journal.frames");
        let mut bytes = fs::read(&path).unwrap();
        let frame = Journal::read(bytes.as_slice(), &plan())
            .unwrap()
            .next_frame(&plan(), staged())
            .unwrap();
        bytes.extend_from_slice(&frame[..cut.min(frame.len() - 1)]);
        fs::write(&path, &bytes).unwrap();
        let original = files(&fixture.0);
        assert_eq!(Operation::inspect(&operation).unwrap(), Inspection::Damaged);
        let mut owner = gate(&fixture.0);
        assert!(owner.admit(SECOND).is_err());
        owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
        assert_eq!(
            Operation::inspect(&operation).unwrap(),
            Inspection::Abandoned
        );
        let mut reopened = Operation::try_open(&operation).unwrap().unwrap();
        assert!(reopened.journal().damage().is_some());
        assert_eq!(reopened.journal().source_bytes(), bytes);
        assert!(reopened.append(staged()).is_err());
        drop(reopened);
        owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
        assert!(owner.admit(OPERATION_ID).is_err());
        assert!(owner.admit(SECOND).is_ok());
        drop(owner);
        let receipt = operation
            .join("cleanup-recovery.frames")
            .canonicalize()
            .unwrap();
        let mut after = files(&fixture.0);
        let saved = after.remove(&receipt).unwrap();
        // Admission above adds a new claim; all old bytes are still retained.
        for (path, bytes) in original {
            assert_eq!(after.get(&path), Some(&bytes));
        }
        assert!(String::from_utf8(saved)
            .unwrap()
            .contains("completed-worker-records-damaged-journal-v1"));

        // Even a change to the previously uninterpreted suffix invalidates the
        // receipt. Recovery binds every original byte, not just the valid prefix.
        bytes.push(b' ');
        fs::write(path, bytes).unwrap();
        let mut owner = gate(&fixture.0);
        assert!(Operation::inspect(&operation).is_err());
        assert!(owner.abandon_cleaned_validation(OPERATION_ID).is_err());
        assert!(owner.admit(SECOND).is_err());
    }
}

#[test]
fn damaged_completion_still_requires_complete_workers_and_a_validating_prefix() {
    for mode in [
        "outstanding-worker",
        "missing-workers",
        "terminal-prefix",
        "prepared-prefix",
    ] {
        let fixture = setup();
        let operation = interrupted(&fixture.0, 1);
        let path = operation.join("journal.frames");
        let mut bytes = fs::read(&path).unwrap();
        match mode {
            "outstanding-worker" => {
                fs::remove_file(operation.join("workers/0000-cleaned.json")).unwrap()
            }
            "missing-workers" => fs::rename(
                operation.join("workers"),
                operation.join("retained-workers"),
            )
            .unwrap(),
            "terminal-prefix" => {
                let frame = Journal::read(bytes.as_slice(), &plan())
                    .unwrap()
                    .next_frame(&plan(), staged())
                    .unwrap();
                bytes.extend_from_slice(&frame);
            }
            "prepared-prefix" => {
                let end = bytes
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == b'\n')
                    .nth(1)
                    .unwrap()
                    .0
                    + 1;
                bytes.truncate(end);
            }
            _ => unreachable!(),
        }
        bytes.extend_from_slice(b"partial");
        fs::write(path, bytes).unwrap();
        let original = files(&fixture.0);
        let mut owner = gate(&fixture.0);
        assert!(
            owner.abandon_cleaned_validation(OPERATION_ID).is_err(),
            "{mode}"
        );
        assert!(owner.admit(SECOND).is_err(), "{mode}");
        drop(owner);
        assert_eq!(files(&fixture.0), original, "{mode}");
    }
}

#[test]
fn recovery_requires_both_leases_and_an_existing_matching_claim() {
    let fixture = setup();
    let operation = interrupted(&fixture.0, 1);
    let mut owner = gate(&fixture.0);
    assert!(Admission::try_open(&fixture.0, PROFILE).unwrap().is_none());
    let lease = Operation::try_open(&operation).unwrap().unwrap();
    assert!(owner.abandon_cleaned_validation(OPERATION_ID).is_err());
    drop(lease);
    assert!(owner.abandon_cleaned_validation(SECOND).is_err());
    let plan = operation.join("plan.json");
    let original = fs::read_to_string(&plan).unwrap();
    fs::write(&plan, original.replace(TARGET, SECOND)).unwrap();
    assert!(owner.abandon_cleaned_validation(OPERATION_ID).is_err());
    fs::write(&plan, original + "\n").unwrap();
    assert!(owner.abandon_cleaned_validation(OPERATION_ID).is_err());
    assert!(!operation.join("cleanup-recovery.frames").exists());
}

#[test]
fn changed_or_partial_recovery_receipts_cannot_bypass_original_history() {
    for mode in [
        "receipt",
        "worker",
        "extra-worker",
        "journal",
        "plan",
        "other-operation",
        "partial-existing",
    ] {
        let fixture = setup();
        let operation = interrupted(&fixture.0, 1);
        let receipt = operation.join("cleanup-recovery.frames");
        let mut owner = gate(&fixture.0);
        if mode == "partial-existing" {
            fs::write(&receipt, b"partial").unwrap();
        } else {
            owner.abandon_cleaned_validation(OPERATION_ID).unwrap();
            match mode {
                "receipt" => {
                    let bytes = fs::read(&receipt).unwrap();
                    fs::write(&receipt, &bytes[..bytes.len() - 1]).unwrap();
                }
                "worker" => {
                    let path = operation.join("workers/0000-cleaned.json");
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.push(b' ');
                    fs::write(path, bytes).unwrap();
                }
                "extra-worker" => {
                    fs::write(operation.join("workers/0001-intent.json"), b"{}").unwrap()
                }
                "journal" => {
                    let path = operation.join("journal.frames");
                    let journal =
                        Journal::read(fs::read(&path).unwrap().as_slice(), &plan()).unwrap();
                    let frame = journal
                        .next_frame(&plan(), failed(Cleanup::Confirmed))
                        .unwrap();
                    OpenOptions::new()
                        .append(true)
                        .open(path)
                        .unwrap()
                        .write_all(&frame)
                        .unwrap();
                }
                "plan" => {
                    let path = operation.join("plan.json");
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.push(b' ');
                    fs::write(path, bytes).unwrap();
                }
                "other-operation" => {
                    fs::copy(
                        &receipt,
                        fixture
                            .0
                            .join("operations")
                            .join(SECOND)
                            .join("cleanup-recovery.frames"),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        drop(owner);
        let original = files(&fixture.0);
        let mut owner = gate(&fixture.0);
        let target = if mode == "other-operation" {
            SECOND
        } else {
            OPERATION_ID
        };
        assert!(owner.abandon_cleaned_validation(target).is_err(), "{mode}");
        assert!(owner.admit(SECOND).is_err(), "{mode}");
        drop(owner);
        assert_eq!(files(&fixture.0), original, "{mode}");
    }
}
