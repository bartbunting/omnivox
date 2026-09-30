//! Private owner-to-worker gate. Executable definitions never enter the speech
//! protocol; the complete frame is validated before CLI/native startup.
use std::io::{Read, Write};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use omnivox_tts::engine_configuration::{LaunchSnapshot, MAX_SNAPSHOT_BYTES};

const MAGIC: &[u8; 7] = b"START1\n";
static SNAPSHOT: OnceLock<LaunchSnapshot> = OnceLock::new();

pub fn snapshot() -> Option<&'static LaunchSnapshot> {
    SNAPSHOT.get()
}

pub fn receive_owned() -> Result<()> {
    let snapshot = read(&mut std::io::stdin().lock())?;
    anyhow::ensure!(
        SNAPSHOT.set(snapshot).is_ok(),
        "worker startup already installed"
    );
    Ok(())
}

pub fn write(output: &mut impl Write, snapshot: &LaunchSnapshot) -> Result<()> {
    let bytes = snapshot.to_bytes()?;
    output.write_all(MAGIC)?;
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}

fn read(input: &mut impl Read) -> Result<LaunchSnapshot> {
    let mut magic = [0; MAGIC.len()];
    input
        .read_exact(&mut magic)
        .context("owner closed before worker startup")?;
    anyhow::ensure!(&magic == MAGIC, "invalid private worker startup version");
    let mut length = [0; 4];
    input
        .read_exact(&mut length)
        .context("incomplete worker startup length")?;
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length > 0 && length <= MAX_SNAPSHOT_BYTES,
        "worker startup exceeds bound"
    );
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .context("incomplete worker startup snapshot")?;
    Ok(LaunchSnapshot::parse(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::engine_configuration::{
        LaunchEnvironment, LoadedConfiguration, Platform, ResolvedConfiguration,
    };
    use std::collections::BTreeMap;
    use std::path::Path;

    fn prepared() -> LaunchSnapshot {
        let resolved = ResolvedConfiguration::resolve(
            LoadedConfiguration::default(),
            Path::new("/nonexistent/omnivox"),
            Platform::native(),
            LaunchEnvironment::from_variables(BTreeMap::new()),
            &BTreeMap::new(),
        )
        .unwrap();
        LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap()
    }

    #[test]
    fn complete_gate_preserves_configuration_and_following_speech_commands() {
        let snapshot = prepared();
        let mut bytes = Vec::new();
        write(&mut bytes, &snapshot).unwrap();
        bytes.extend_from_slice(b"q {after startup}\nd\n");
        let mut input = bytes.as_slice();
        let received = read(&mut input).unwrap();
        assert_eq!(received.to_bytes().unwrap(), snapshot.to_bytes().unwrap());
        assert_eq!(input, b"q {after startup}\nd\n");
    }

    #[test]
    fn partial_oversized_and_legacy_gates_do_not_construct_a_snapshot() {
        let mut bytes = Vec::new();
        write(&mut bytes, &prepared()).unwrap();
        for end in [0, 6, 7, 10, bytes.len() - 1] {
            assert!(read(&mut &bytes[..end]).is_err());
        }
        assert!(read(&mut b"START\nq hello\n".as_slice()).is_err());
        for length in [0, MAX_SNAPSHOT_BYTES as u32 + 1, u32::MAX] {
            let mut invalid = MAGIC.to_vec();
            invalid.extend_from_slice(&length.to_be_bytes());
            assert!(read(&mut invalid.as_slice()).is_err());
        }
        let mut invalid = MAGIC.to_vec();
        invalid.extend_from_slice(&2u32.to_be_bytes());
        invalid.extend_from_slice(b"{}");
        assert!(read(&mut invalid.as_slice()).is_err());
    }
}
