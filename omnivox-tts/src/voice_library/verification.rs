//! Content verification uses bounded memory and never modifies user files.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use sha2::{Digest, Sha256};

use super::{AssetFile, LibraryError, PackageRevision, ProviderOverrides, RuntimeLibrary};

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl RuntimeLibrary {
    /// SHA-256 of the exact generation input, including whitespace and key order.
    pub fn sha256(&self) -> String {
        digest(self.source_bytes())
    }

    /// Verify the assets this generation will supply after provider overrides.
    /// This does not load native code or establish native compatibility. Helpers
    /// must recheck before opening assets because user-owned files can change.
    pub fn verify_assets(&self, overrides: ProviderOverrides) -> Result<(), LibraryError> {
        if !overrides.piper {
            if let Some(piper) = &self.document().piper {
                for model in &piper.models {
                    model.model.open_verified()?;
                    model.config.open_verified()?;
                }
            }
        }
        if !overrides.flite {
            if let Some(flite) = &self.document().flite {
                for voice in &flite.files {
                    voice.file.open_verified()?;
                }
            }
        }
        Ok(())
    }
}

impl PackageRevision {
    /// Digest identifying the contract's sorted, compact file-set metadata.
    /// This does not read files or establish that native validation is current.
    pub fn file_set_sha256(&self) -> Result<String, LibraryError> {
        Ok(digest(&self.file_set_bytes()?))
    }
}

impl AssetFile {
    /// Check type, exact size and content hash, returning the verified handle
    /// rewound for reading. Native APIs which reopen by path must call this just
    /// before loading. Neither case can make a concurrently writable file immutable.
    pub fn open_verified(&self) -> Result<File, LibraryError> {
        let verify = || -> Result<File, String> {
            // Reject known directories/devices/FIFOs before an open could block.
            // Check the opened handle too; a path may have changed in between.
            let metadata = std::fs::metadata(&self.path).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err("path is not a regular file".to_owned());
            }
            let mut file = File::open(&self.path).map_err(|e| e.to_string())?;
            let before = file.metadata().map_err(|e| e.to_string())?;
            if !before.is_file() || before.len() != self.bytes {
                return Err("asset type or size differs from the library".to_owned());
            }
            self.verify_reader(&mut file)?;
            let after = file.metadata().map_err(|e| e.to_string())?;
            if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
                return Err("asset changed during verification".to_owned());
            }
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            Ok(file)
        };
        verify().map_err(|reason| LibraryError::Asset {
            path: self.path.clone(),
            reason,
        })
    }

    fn verify_reader(&self, reader: &mut impl Read) -> Result<(), String> {
        let mut hash = Sha256::new();
        let mut remaining = self.bytes;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let limit = remaining.min(buffer.len() as u64) as usize;
            let count = match reader.read(&mut buffer[..limit]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|e| e.to_string())?,
            };
            if count == 0 {
                return Err("asset ended before its declared size".to_owned());
            }
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0];
        loop {
            match reader.read(&mut extra) {
                Ok(0) => break,
                Ok(_) => return Err("asset exceeds its declared size".to_owned()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        if hex(&hash.finalize()) != self.sha256 {
            return Err("asset SHA-256 differs from the library".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
