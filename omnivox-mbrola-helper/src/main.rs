//! Explicitly staged MBROLA experiment; not part of generic release payloads.
mod engine;
mod process;

use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // On Windows this job contains the helper before it can create a child.
    // Children inherit membership atomically, including on forced helper death.
    let _ownership = process::own_helper()?;
    let root = std::env::current_exe()?
        .parent()
        .ok_or("helper has no parent directory")?
        .to_path_buf();
    if std::env::args_os().len() != 1 {
        return Err("this prototype reads only its adjacent prototype.json".into());
    }
    let engine = Arc::new(engine::MbrolaEngine::new(root));
    omnivox_helper_host::run_stdio(
        engine,
        "Omnivox MBROLA prototype",
        env!("CARGO_PKG_VERSION"),
    )?;
    Ok(())
}
