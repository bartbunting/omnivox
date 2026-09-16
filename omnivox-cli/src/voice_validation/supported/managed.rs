//! Development execution under both profile and operation ownership.
use super::{evidence, owned, validate, Options};
use anyhow::{Context, Result};
use omnivox_tts::voice_library::operations::{
    Admission, Cleanup, ExecutionRecords, Transition, ValidationState,
};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

struct Recorder<'a>(&'a mut ExecutionRecords);
impl owned::Recorder for Recorder<'_> {
    fn starting(&mut self, command: &Command) -> Result<()> {
        let program = command
            .get_program()
            .to_str()
            .context("non-UTF-8 validator path")?;
        let arguments = command
            .get_args()
            .map(|arg| {
                arg.to_str()
                    .map(str::to_owned)
                    .context("non-UTF-8 validation argument")
            })
            .collect::<Result<Vec<_>>>()?;
        self.0.starting(program, &arguments)?;
        Ok(())
    }
    fn owned(&mut self, pid: u32) -> Result<()> {
        self.0.owned(pid)?;
        Ok(())
    }
    fn cleaned(&mut self) -> Result<()> {
        self.0.cleaned()?;
        Ok(())
    }
}

pub(super) fn run(args: &[String]) -> Result<()> {
    anyhow::ensure!(
        args.len() == 4,
        "expected internal supervisor ROOT PROFILE_UUID OPERATION_UUID"
    );
    super::manager::await_start()?;
    let mut admission = Admission::try_open(Path::new(&args[1]), &args[2])?
        .context("profile validation is already owned")?;
    let mut admitted = admission.admit(&args[3])?;
    let plan = admitted.operation().plan().clone();
    let path = admitted.operation().path().to_path_buf();
    let mut validating = false;
    let mut records: Option<ExecutionRecords> = None;
    let mut cancelled = None;
    let result = (|| -> Result<()> {
        let request = plan.document();
        anyhow::ensure!(
            request.platform == std::env::consts::OS,
            "validation plan is for another platform"
        );
        anyhow::ensure!(
            Path::new(&request.validator_path).canonicalize()?
                == std::env::current_exe()?.canonicalize()?,
            "run the exact validator named by the frozen plan"
        );
        evidence::check_environment()?;
        let mut helpers = BTreeMap::new();
        for (engine, path) in &request.helpers {
            let path = PathBuf::from(path)
                .canonicalize()
                .context("could not locate planned helper")?;
            anyhow::ensure!(path.is_file(), "planned helper must be a regular file");
            helpers.insert(engine.clone(), path);
        }
        let options = Options {
            path: path.join("input-generation.json"),
            helpers,
            timeout: Duration::from_secs(request.timeout_seconds),
            memory: usize::try_from(request.memory_bytes)
                .context("memory budget exceeds this host's range")?,
            report: Some(path.join("validation-evidence.json")),
            check_report: None,
        };
        owned::initialize()?;
        cancelled = Some(owned::cancellation_input()?);
        let cancellation = cancelled.as_ref().unwrap();
        anyhow::ensure!(
            !cancellation.load(Ordering::Acquire),
            "voice validation cancelled"
        );
        admitted.append(Transition {
            state: ValidationState::Validating,
            cleanup: Cleanup::Unconfirmed,
            evidence_sha256: None,
            detail: None,
        })?;
        validating = true;
        records = Some(ExecutionRecords::create(&admitted)?);
        let mut input = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&options.path)?;
        input.write_all(plan.generation().source_bytes())?;
        input.sync_all()?;
        drop(input);
        let records = records.as_mut().unwrap();
        let report = validate(
            &options,
            plan.generation(),
            cancellation,
            &mut Recorder(records),
        )?
        .context("managed validation produced no evidence")?;
        let evidence = records.bind(&plan, &report)?;
        anyhow::ensure!(
            !cancellation.load(Ordering::Acquire),
            "voice validation cancelled"
        );
        let digest = evidence.save(&admitted)?;
        anyhow::ensure!(
            !cancellation.load(Ordering::Acquire),
            "voice validation cancelled; retained evidence is not a staged result"
        );
        admitted.append(Transition {
            state: ValidationState::Staged,
            cleanup: Cleanup::Confirmed,
            evidence_sha256: Some(digest),
            detail: None,
        })?;
        Ok(())
    })();
    if let Err(error) = result {
        let pending = records.as_ref().is_some_and(ExecutionRecords::pending);
        let is_cancelled = cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        let transition = Transition {
            state: if pending {
                ValidationState::RecoveryFailed
            } else if is_cancelled {
                ValidationState::Cancelled
            } else {
                ValidationState::Failed
            },
            cleanup: if pending {
                Cleanup::Unconfirmed
            } else if validating {
                Cleanup::Confirmed
            } else {
                Cleanup::NotStarted
            },
            evidence_sha256: None,
            detail: Some(
                format!("Validation stopped: {error}")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect(),
            ),
        };
        if let Err(record_error) = admitted.append(transition) {
            eprintln!("Could not record terminal state; retained operation requires inspection: {record_error}");
        }
        return Err(error)
            .with_context(|| format!("validation operation retained at {}", path.display()));
    }
    println!("Validation operation {} staged; native cleanup confirmed; installation and activation remain separate", plan.document().operation_id);
    Ok(())
}
