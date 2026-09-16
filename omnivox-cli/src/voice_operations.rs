//! Development preparation/inspection only. Never executes a saved request.
use anyhow::{Context, Result};
use omnivox_tts::voice_library::operations::{Admission, Inspection, Operation, ValidationPlan};
use std::fs::File;
use std::path::Path;

pub fn requested(args: &[String]) -> bool {
    args.first().is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "--prepare-voice-validation"
                | "--inspect-voice-operation"
                | "--prepare-voice-admission"
                | "--inspect-voice-admission"
        )
    })
}
pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("--prepare-voice-admission") => {
            anyhow::ensure!(
                args.len() == 4,
                "expected --prepare-voice-admission ROOT TARGET_UUID PROFILE_UUID"
            );
            Admission::create(Path::new(&args[1]), &args[2], &args[3])?;
            println!("Prepared profile validation admission; no native work started");
        }
        Some("--inspect-voice-admission") => {
            anyhow::ensure!(
                args.len() == 3,
                "expected --inspect-voice-admission ROOT PROFILE_UUID"
            );
            match Admission::try_open(Path::new(&args[1]), &args[2])? {
                None => println!("Profile validation is owned; no recovery action taken"),
                Some(admission) => {
                    let entries = admission.inspect()?;
                    println!(
                        "Target {}; {} retained operation claims",
                        admission.target_id(),
                        entries.len()
                    );
                    for entry in entries {
                        println!("{}: {:?}", entry.operation_id, entry.state);
                    }
                    println!("Admission still requires checking the requested operation; inspection grants no cleanup or activation authority");
                }
            }
        }
        Some("--prepare-voice-validation") => {
            anyhow::ensure!(
                args.len() == 3,
                "expected --prepare-voice-validation PLAN_JSON OPERATIONS_DIRECTORY"
            );
            let plan = ValidationPlan::read(File::open(&args[1])?)?;
            let operation = Operation::create(Path::new(&args[2]), plan).context(
                "could not prepare validation operation; existing or partial files are retained",
            )?;
            println!(
                "Prepared validation operation {} at {}; no native work started",
                operation.plan().document().operation_id,
                operation.path().display()
            );
        }
        Some("--inspect-voice-operation") => {
            anyhow::ensure!(
                args.len() == 2,
                "expected --inspect-voice-operation OPERATION_DIRECTORY"
            );
            let status = Operation::inspect(Path::new(&args[1]))
                .context("operation could not be inspected; retain its files for recovery")?;
            println!("{}", match status {
                Inspection::Busy => "Operation is owned; no recovery action taken",
                Inspection::Prepared => "Prepared; no native work recorded; admission checks are still required",
                Inspection::Interrupted => "Interrupted validation; cleanup is unconfirmed; operation reuse is blocked",
                Inspection::Staged => "Validation staged with recorded cleanup and evidence; installation and activation remain separate",
                Inspection::Cancelled => "Validation cancelled with no outstanding native work recorded",
                Inspection::Failed => "Validation failed with no outstanding native work recorded",
                Inspection::RecoveryFailed => "Cleanup failed; operation reuse is blocked",
                Inspection::Damaged => "Incomplete or damaged journal; operation reuse is blocked; retained for recovery",
            });
        }
        _ => anyhow::bail!("unknown voice-operation command"),
    }
    Ok(())
}
