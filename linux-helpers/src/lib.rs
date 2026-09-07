// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// Linux adapters for user-installed runtimes. Each binary owns only one engine.

#[cfg(target_os = "linux")]
mod dectalk;
#[cfg(target_os = "linux")]
mod eloquence;
#[cfg(target_os = "linux")]
mod markers;
#[cfg(target_os = "linux")]
mod native;

#[cfg(target_os = "linux")]
pub fn run(id: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    let engine = std::sync::Arc::new(native::Engine::new(id));
    omnivox_helper_host::run_stdio(
        engine,
        format!("Omnivox Linux {id} helper"),
        env!("CARGO_PKG_VERSION"),
    )?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn run(_: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    Err("These helpers require Linux".into())
}
