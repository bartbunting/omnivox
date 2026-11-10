//! Private, inherited creation channel. It never carries executable definitions.
use omnivox_tts::engine_configuration::LaunchEnvironment;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub request_id: u64,
    pub command: String,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub launch: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub request_id: u64,
    pub state: String,
    pub handles: Option<[usize; 3]>,
    pub error: Option<String>,
}

pub(super) fn identity(
    engine: &str,
    program: &Path,
    args: &[OsString],
    environment: &LaunchEnvironment,
) -> io::Result<String> {
    let program: Vec<u16> = program.as_os_str().encode_wide().collect();
    let args: Vec<Vec<u16>> = args.iter().map(|arg| arg.encode_wide().collect()).collect();
    let bytes =
        serde_json::to_vec(&(engine, program, args, environment)).map_err(io::Error::other)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(io::Error::other("helper launch exceeds private bound"));
    }
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
