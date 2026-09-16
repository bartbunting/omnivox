//! Private, local stdio service records. Never carried by the remote socket.
use super::installation::ActivationCandidate;
use super::operations::storage::{new_file, open_file, ordinary};
use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "OMNIVOX-LOCAL ";
pub const MAX_LINE: usize = 2 * MAX_RUNTIME_BYTES;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: u64,
    pub command: String,
    #[serde(default)]
    pub worker: String,
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub generation: String,
    #[serde(default)]
    pub expected_sha256: String,
    #[serde(default)]
    pub plan_json: String,
    #[serde(default)]
    pub proofs_json: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub piper: bool,
    #[serde(default)]
    pub flite: bool,
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub revision: String,
}
impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, LibraryError> {
        let request: Self = decode(line, MAX_LINE)?;
        require(
            request.request_id > 0,
            "local request requires a positive ID",
        )?;
        Ok(request)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Catalogue {
        catalogue: super::catalogue::CatalogueDocument,
    },
    Acquisition {
        progress: super::acquisition::Progress,
    },
    AcquisitionStatus {
        events: Vec<super::acquisition::Progress>,
    },
    Host {
        root: String,
        target_id: String,
        profile_id: String,
    },
    Owner {
        worker: String,
        startup: String,
        startup_sha256: String,
        configuration: Option<VoiceLibraryConfiguration>,
        retired: bool,
        startup_error: Option<String>,
    },
    Retired {
        worker: String,
    },
    Snapshot {
        startup: String,
        startup_sha256: String,
        configuration: VoiceLibraryConfiguration,
    },
    Library {
        index: IndexDocument,
        sha256: String,
        active: Option<ActivePointer>,
    },
    Candidate {
        candidate: ActivationCandidate,
        path: String,
    },
    State {
        state: String,
    },
    Committed {
        configuration: VoiceLibraryConfiguration,
    },
    Error {
        message: String,
    },
}
impl Reply {
    pub fn line(&self, request_id: u64) -> Result<String, LibraryError> {
        #[derive(Serialize)]
        struct Envelope<'a> {
            request_id: u64,
            #[serde(flatten)]
            reply: &'a Reply,
        }
        let json = serde_json::to_string(&Envelope {
            request_id,
            reply: self,
        })?;
        require(json.len() < MAX_LINE, "local reply exceeds bound")?;
        Ok(format!("{PREFIX}{json}\n"))
    }
}

/// Native-selected identity persists independently of executable installations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    schema_version: u32,
    pub target_id: String,
    pub profile_id: String,
    #[serde(skip)]
    pub root: PathBuf,
}
impl Host {
    pub fn open_default() -> Result<Self, LibraryError> {
        let root = if let Some(root) = std::env::var_os("OMNIVOX_VOICE_ROOT") {
            PathBuf::from(root)
        } else if cfg!(windows) {
            PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .ok_or(LibraryError::Invalid("LOCALAPPDATA is missing"))?,
            )
            .join("Emacsvox")
            .join("Omnivox")
            .join("voices")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or(
                    PathBuf::from(
                        std::env::var_os("HOME").ok_or(LibraryError::Invalid("HOME is missing"))?,
                    )
                    .join(".local/share"),
                )
                .join("emacsvox/omnivox/voices")
        };
        Self::open(&root)
    }
    pub fn open(root: &Path) -> Result<Self, LibraryError> {
        require(root.is_absolute(), "voice root must be native and absolute")?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(root)?;
        ordinary(root, false)?;
        let root = root.canonicalize()?;
        let lock_path = root.join("initialize.lock");
        let lease = match new_file(&lock_path) {
            Ok(file) => file,
            Err(LibraryError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                open_file(&lock_path, true)?
            }
            Err(error) => return Err(error),
        };
        lease.lock()?;
        let host_path = root.join("host.json");
        let mut host: Self = if host_path.try_exists()? {
            decode(&read_bounded(open_file(&host_path, false)?, 4096)?, 4096)?
        } else {
            // Partial initialization is retained. Do not guess which files are
            // safe to overwrite after an interrupted first setup.
            let host = Self {
                schema_version: 1,
                target_id: new_uuid()?,
                profile_id: new_uuid()?,
                root: root.clone(),
            };
            private_directory(&root.join("operations"))?;
            private_directory(&root.join("profiles"))?;
            private_directory(&root.join("profiles").join(&host.profile_id))?;
            private_directory(&root.join("sessions"))?;
            drop(operations::Admission::create(
                &root,
                &host.target_id,
                &host.profile_id,
            )?);
            drop(installation::Profile::initialize(
                &root,
                &host.profile_id,
                &new_uuid()?,
            )?);
            save(&host_path, &serde_json::to_vec(&host)?)?;
            host
        };
        require(host.schema_version == 1, "unsupported native host identity")?;
        uuid(&host.target_id)?;
        uuid(&host.profile_id)?;
        host.root = root;
        lease.unlock()?;
        Ok(host)
    }
    pub fn profile(&self) -> Result<installation::Profile, LibraryError> {
        let profile = installation::Profile::open(&self.root, &self.profile_id)?;
        require(
            profile.index().document().target_id == self.target_id,
            "native host identity changed",
        )?;
        Ok(profile)
    }
    pub fn reply(&self) -> Reply {
        Reply::Host {
            root: self.root.to_string_lossy().into(),
            target_id: self.target_id.clone(),
            profile_id: self.profile_id.clone(),
        }
    }
}

/// Resolved native inputs, saved once by the owner before opening START.
/// Full environment stays in this private local record, never in speech logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Startup {
    pub executable: AssetFile,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub configuration: Option<VoiceLibraryConfiguration>,
}
impl Startup {
    pub fn capture(host: &Host) -> Result<Self, LibraryError> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let mut environment: BTreeMap<String, String> = std::env::vars_os()
            .map(|(key, value)| {
                Ok((
                    key.into_string()
                        .map_err(|_| LibraryError::Invalid("non-UTF-8 startup environment"))?,
                    value
                        .into_string()
                        .map_err(|_| LibraryError::Invalid("non-UTF-8 startup environment"))?,
                ))
            })
            .collect::<Result<_, LibraryError>>()?;
        environment.remove("OMNIVOX_REMOTE_WORKER");
        environment.remove("OMNIVOX_OWNED_STARTUP");
        environment.remove("OMNIVOX_OWNED_LIBRARY");
        if !environment.contains_key("OMNIVOX_VOICE_LIBRARY") {
            let profile = host.profile()?;
            if let Some(active) = profile.active_json()? {
                let active = ActivePointer::parse(active.as_bytes())?;
                environment.insert(
                    "OMNIVOX_VOICE_LIBRARY".into(),
                    profile
                        .generation_path(&active.generation_id)
                        .to_string_lossy()
                        .into(),
                );
            }
        }
        let mut startup = Self {
            executable: identify(&executable)?,
            arguments: Vec::new(),
            working_directory: std::env::current_dir()?,
            environment,
            configuration: None,
        };
        startup.configuration = startup.library()?.map(|library| library.configuration());
        Ok(startup)
    }
    fn library(&self) -> Result<Option<RuntimeLibrary>, LibraryError> {
        self.environment
            .get("OMNIVOX_VOICE_LIBRARY")
            .map(|path| {
                RuntimeLibrary::read(
                    open_file(Path::new(path), false)?,
                    if cfg!(windows) {
                        HostPlatform::Windows
                    } else {
                        HostPlatform::Posix
                    },
                )
            })
            .transpose()
    }
    pub fn candidate(&mut self, path: &Path) -> Result<(), LibraryError> {
        // The launcher labels its fallback separately from explicit settings.
        // Applying a managed Piper set replaces that fallback only. User file
        // overrides remain visible and must be resolved before retirement.
        let library = RuntimeLibrary::read(
            open_file(path, false)?,
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )?;
        if library.document().piper.is_some()
            && self.environment.contains_key("OMNIVOX_PIPER_MODEL")
            && self.environment.get("OMNIVOX_PIPER_MODEL")
                == self.environment.get("OMNIVOX_LAUNCHER_PIPER_DEFAULT")
        {
            self.environment.remove("OMNIVOX_PIPER_MODEL");
        }
        self.environment.insert(
            "OMNIVOX_VOICE_LIBRARY".into(),
            path.to_string_lossy().into(),
        );
        self.configuration = self.library()?.map(|library| library.configuration());
        Ok(())
    }
    pub fn read(path: &Path, expected: &str) -> Result<Self, LibraryError> {
        sha256(expected)?;
        let bytes = read_bounded(open_file(path, false)?, MAX_RUNTIME_BYTES)?;
        require(
            verification::digest(&bytes) == expected,
            "native startup snapshot changed",
        )?;
        let startup: Self = decode(&bytes, MAX_RUNTIME_BYTES)?;
        startup.verify()?;
        Ok(startup)
    }
    pub fn verify(&self) -> Result<(), LibraryError> {
        self.executable.open_verified()?;
        require(
            self.library()?.map(|library| library.configuration()) == self.configuration,
            "startup generation changed",
        )
    }
    pub fn save(&self, host: &Host, worker: &str) -> Result<(PathBuf, String), LibraryError> {
        uuid(worker)?;
        let path = host.root.join("sessions").join(format!("{worker}.json"));
        let bytes = serde_json::to_vec(self)?;
        require(
            bytes.len() <= MAX_RUNTIME_BYTES,
            "startup snapshot exceeds bound",
        )?;
        save(&path, &bytes)?;
        Ok((path, verification::digest(&bytes)))
    }
}

fn identify(path: &Path) -> Result<AssetFile, LibraryError> {
    use sha2::{Digest, Sha256};
    let mut input = open_file(path, false)?;
    let bytes = input.metadata()?.len();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let file = AssetFile {
        path: path.to_string_lossy().into(),
        bytes,
        sha256: hash.finalize().iter().map(|b| format!("{b:02x}")).collect(),
    };
    file.open_verified()?;
    Ok(file)
}
fn private_directory(path: &Path) -> Result<(), LibraryError> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)?;
    Ok(())
}
fn save(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let mut file = new_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}

pub fn new_uuid() -> Result<String, LibraryError> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                provider: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        // SAFETY: a valid 16-byte writable buffer, system-preferred RNG.
        require(
            unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, 2) } >= 0,
            "native random source failed",
        )?;
    }
    #[cfg(not(any(unix, windows)))]
    return Err(LibraryError::Invalid("unsupported native UUID source"));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("omnivox-local-{}", new_uuid().unwrap())))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn native_identity_persists_and_does_not_reset_partial_initialization() {
        let fixture = Fixture::new();
        let first = Host::open(&fixture.0).unwrap();
        let second = Host::open(&fixture.0).unwrap();
        assert_eq!(first.target_id, second.target_id);
        assert_eq!(first.profile_id, second.profile_id);
        let profile = first.profile().unwrap();
        assert!(second.profile().is_err());
        drop(profile);
        fs::remove_file(fixture.0.join("host.json")).unwrap();
        assert!(Host::open(&fixture.0).is_err());
        assert!(fixture
            .0
            .join("profiles")
            .join(&first.profile_id)
            .join("index.json")
            .exists());
    }

    #[test]
    fn candidate_replaces_only_launcher_fallback_and_freezes_original_startup() {
        let fixture = Fixture::new();
        let host = Host::open(&fixture.0).unwrap();
        let profile = host.profile().unwrap();
        let generation = new_uuid().unwrap();
        profile
            .stage_activation(&generation, true, false, &profile.index_sha256())
            .unwrap();
        let mut startup = Startup {
            executable: identify(&std::env::current_exe().unwrap()).unwrap(),
            arguments: Vec::new(),
            working_directory: fixture.0.clone(),
            environment: BTreeMap::from([
                ("OMNIVOX_PIPER_MODEL".into(), "launcher-default.onnx".into()),
                (
                    "OMNIVOX_LAUNCHER_PIPER_DEFAULT".into(),
                    "launcher-default.onnx".into(),
                ),
            ]),
            configuration: None,
        };
        let (path, hash) = startup.save(&host, &new_uuid().unwrap()).unwrap();
        startup
            .candidate(&profile.generation_path(&generation))
            .unwrap();
        assert!(!startup.environment.contains_key("OMNIVOX_PIPER_MODEL"));
        let mut restored = Startup::read(&path, &hash).unwrap();
        assert!(restored.configuration.is_none());
        assert_eq!(
            restored.environment["OMNIVOX_PIPER_MODEL"],
            "launcher-default.onnx"
        );
        restored.environment.insert(
            "OMNIVOX_PIPER_MODEL".into(),
            "explicit-user-file.onnx".into(),
        );
        restored
            .candidate(&profile.generation_path(&generation))
            .unwrap();
        assert_eq!(
            restored.environment["OMNIVOX_PIPER_MODEL"],
            "explicit-user-file.onnx"
        );
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(&path, bytes).unwrap();
        assert!(Startup::read(&path, &hash).is_err());
    }
}
