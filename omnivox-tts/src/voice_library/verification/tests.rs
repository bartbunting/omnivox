use super::*;
use crate::voice_library::{HostPlatform, LibraryIndex};
use std::io::{self, Cursor};

const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn content_verification_checks_exact_bytes_with_bounded_reads() {
    let mut asset = AssetFile {
        path: "/unused".into(),
        bytes: 3,
        sha256: ABC.into(),
    };
    asset.verify_reader(&mut Cursor::new(b"abc")).unwrap();
    for content in [b"abd".as_slice(), b"ab", b"abcd"] {
        assert!(asset.verify_reader(&mut Cursor::new(content)).is_err());
    }

    struct LargeInput {
        remaining: usize,
        interrupted: bool,
    }
    impl Read for LargeInput {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(buf.len() <= 64 * 1024);
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let count = buf.len().min(self.remaining).min(701);
            buf[..count].fill(b'a');
            self.remaining -= count;
            Ok(count)
        }
    }
    asset.bytes = 1_000_000;
    asset.sha256 = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0".into();
    asset
        .verify_reader(&mut LargeInput {
            remaining: 1_000_000,
            interrupted: false,
        })
        .unwrap();
    // A producer with arbitrarily more data is cut off at the declared size + 1.
    let mut growing = LargeInput {
        remaining: usize::MAX,
        interrupted: false,
    };
    assert!(asset
        .verify_reader(&mut growing)
        .unwrap_err()
        .contains("exceeds"));
    assert_eq!(growing.remaining, usize::MAX - 1_000_001);
}

#[test]
fn native_file_verification_rewinds_and_rejects_replaced_assets() {
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root =
        Fixture(std::env::temp_dir().join(format!("omnivox-asset-hash-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    let path = root.0.join("voice");
    std::fs::write(&path, b"abc").unwrap();
    let asset = AssetFile {
        path: path.to_str().unwrap().to_owned(),
        bytes: 3,
        sha256: ABC.into(),
    };
    let mut content = String::new();
    asset
        .open_verified()
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "abc");
    std::fs::write(&path, b"abd").unwrap();
    assert!(asset
        .open_verified()
        .unwrap_err()
        .to_string()
        .contains("SHA-256"));
    std::fs::write(&path, b"ab").unwrap();
    assert!(asset
        .open_verified()
        .unwrap_err()
        .to_string()
        .contains("size"));
    std::fs::remove_file(&path).unwrap();
    assert!(asset.open_verified().is_err());
    std::fs::create_dir(&path).unwrap();
    assert!(asset
        .open_verified()
        .unwrap_err()
        .to_string()
        .contains("regular file"));
}

#[test]
fn generation_digest_preserves_source_and_skips_only_overridden_assets() {
    let examples: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/protocol-fixtures/voice-library-v1.json"
    ))
    .unwrap();
    let mut generation = examples["runtime_after_successful_validation"].clone();
    generation["flite"] = serde_json::json!({"builtin_slt":false,"files":[{
        "physical_id":"flitevox:missing", "display_name":"Missing", "language":null,
        "file":{"path":"C:\\missing.flitevox","bytes":3,"sha256":ABC}
    }]});
    let source = serde_json::to_vec(&generation).unwrap();
    let original = RuntimeLibrary::parse(&source, HostPlatform::Windows).unwrap();
    let mut padded = source.clone();
    padded.push(b'\n');
    let padded = RuntimeLibrary::parse(&padded, HostPlatform::Windows).unwrap();
    assert_eq!(original.document(), padded.document());
    assert_ne!(original.sha256(), padded.sha256());
    assert!(original
        .verify_assets(ProviderOverrides::default())
        .is_err());
    assert!(original
        .verify_assets(ProviderOverrides {
            piper: true,
            flite: false
        })
        .is_err());
    assert!(original
        .verify_assets(ProviderOverrides {
            piper: false,
            flite: true
        })
        .is_err());
    original
        .verify_assets(ProviderOverrides {
            piper: true,
            flite: true,
        })
        .unwrap();
    let index = LibraryIndex::parse(
        &serde_json::to_vec(&examples["index_before_validation"]).unwrap(),
        HostPlatform::Windows,
    )
    .unwrap();
    let revision = &index.document().packages[0];
    let mut reversed = revision.clone();
    reversed.files.reverse();
    assert_eq!(
        revision.file_set_sha256().unwrap(),
        reversed.file_set_sha256().unwrap()
    );
    reversed.files[0].bytes += 1;
    assert_ne!(
        revision.file_set_sha256().unwrap(),
        reversed.file_set_sha256().unwrap()
    );
}
