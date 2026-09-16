//! A retained profile lease spans preflight, paired restart and publication.
use super::*;

/// Local provider transaction. Dropping this object never marks unfinished work
/// successful or clears its journal. Recovery of interrupted Apply is explicit.
pub struct Activation {
    profile: Profile,
    candidate: ActivationCandidate,
    path: PathBuf,
    sequence: usize,
    state: String,
    poisoned: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    sequence: usize,
    state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    role: String,
    worker: String,
    ready: bool,
    negotiated: bool,
    inventory_generation: u64,
    request_id: u64,
    status: crate::control::ControlResponseEnvelope,
}

impl Profile {
    /// Retain the native lease and exact reviewed plan before native preflight.
    pub fn begin_activation(
        self,
        operation: &str,
        generation: &str,
        plan_json: &str,
    ) -> Result<Activation, LibraryError> {
        uuid(operation)?;
        let _: serde_json::Value = decode(plan_json.as_bytes(), MAX_RUNTIME_BYTES)?;
        let candidate = self.activation_candidate(generation)?;
        for entry in self.admission.inspect()? {
            require(
                matches!(
                    entry.state,
                    super::super::operations::Inspection::Staged
                        | super::super::operations::Inspection::Cancelled
                        | super::super::operations::Inspection::Failed
                        | super::super::operations::Inspection::Abandoned
                ),
                "unresolved native validation blocks Apply",
            )?;
        }
        let parent = self.path.join("activations");
        if !parent.try_exists()? {
            directory(&parent)?;
        }
        ordinary(&parent, false)?;
        for entry in fs::read_dir(&parent)? {
            let path = entry?.path();
            ordinary(&path, false)?;
            let completion: Record = decode(
                &read_bounded(open_file(&path.join("completed.json"), false)?, 4096)?,
                4096,
            )?;
            require(
                matches!(
                    completion.state.as_str(),
                    "succeeded" | "rolled-back" | "cancelled" | "failed"
                ),
                "unresolved Apply blocks another activation",
            )?;
        }
        let path = parent.join(operation);
        directory(&path)?;
        save_new(&path.join("plan.json"), plan_json.as_bytes())?;
        save_new(&path.join("candidate.json"), &candidate.to_bytes()?)?;
        let mut activation = Activation {
            profile: self,
            candidate,
            path,
            sequence: 0,
            state: "prepared".into(),
            poisoned: false,
        };
        activation.record("prepared")?;
        Ok(activation)
    }
}

impl Activation {
    pub fn candidate(&self) -> &ActivationCandidate {
        &self.candidate
    }
    pub fn generation_path(&self) -> PathBuf {
        self.profile
            .generation_path(&self.candidate.configuration.generation_id)
    }
    pub fn recheck(&self) -> Result<(), LibraryError> {
        require(!self.poisoned, "Apply journal outcome is uncertain")?;
        let current = self
            .profile
            .activation_candidate(&self.candidate.configuration.generation_id)?;
        require(
            current.to_bytes()? == self.candidate.to_bytes()?,
            "Apply candidate changed",
        )
    }
    fn record(&mut self, state: &str) -> Result<(), LibraryError> {
        require(!self.poisoned, "Apply journal outcome is uncertain")?;
        let record = Record {
            sequence: self.sequence,
            state: state.into(),
        };
        self.poisoned = true;
        save_new(
            &self.path.join(format!("{:04}.json", self.sequence)),
            &serde_json::to_vec(&record)?,
        )?;
        self.sequence += 1;
        self.state = state.into();
        self.poisoned = false;
        Ok(())
    }
    pub fn activating(&mut self) -> Result<(), LibraryError> {
        require(self.state == "prepared", "Apply is not prepared")?;
        self.recheck()?;
        self.record("activating")
    }
    pub fn rolling_back(&mut self) -> Result<(), LibraryError> {
        require(
            self.state == "activating",
            "Apply cannot roll back after publication",
        )?;
        self.recheck()?;
        self.record("rolling-back")
    }
    /// The local client must first verify ordinary readiness and correlated
    /// status of both owned lanes. These receipts are retained for inspection;
    /// this method cannot infer process ownership from caller-supplied JSON.
    pub fn commit(&mut self, proofs_json: &str) -> Result<(), LibraryError> {
        require(self.state == "activating", "Apply is not activating")?;
        let proofs: Vec<Proof> = decode(proofs_json.as_bytes(), MAX_RUNTIME_BYTES)?;
        require(proofs.len() == 2, "Apply requires both lane proofs")?;
        require(
            proofs[0].role == "speaker"
                && proofs[1].role == "notification"
                && proofs[0].worker != proofs[1].worker,
            "Apply proof roles or workers differ",
        )?;
        let mut statuses = Vec::new();
        for proof in &proofs {
            uuid(&proof.worker)?;
            require(
                proof.ready
                    && proof.negotiated
                    && proof.request_id > 0
                    && proof.status.protocol_version == 1
                    && proof.status.request_id == Some(proof.request_id),
                "Apply proof is not ready or correlated",
            )?;
            let crate::control::ControlResponse::VoiceLibraryStatusV1(status) =
                &proof.status.response
            else {
                return Err(LibraryError::Invalid(
                    "Apply proof is not voice-library status",
                ));
            };
            require(
                status.configuration.as_ref() == Some(&self.candidate.configuration)
                    && status.inventory_generation == proof.inventory_generation,
                "Apply proof configuration or inventory differs",
            )?;
            statuses.push(status);
        }
        require(
            statuses[0].eligible_voices == statuses[1].eligible_voices
                && statuses[0].overridden_engines == statuses[1].overridden_engines,
            "Apply pair eligibility differs",
        )?;
        self.recheck()?;
        save_new(
            &self.path.join("verified-pair.json"),
            proofs_json.as_bytes(),
        )?;
        let configuration = &self.candidate.configuration;
        let pointer = ActivePointer {
            schema_version: 1,
            target_id: configuration.target_id.clone(),
            profile_id: configuration.profile_id.clone(),
            generation_id: configuration.generation_id.clone(),
            sha256: configuration.sha256.clone(),
        };
        let pending = self
            .profile
            .path
            .join(format!("active-next-{}.json", configuration.generation_id));
        save_new(&pending, &serde_json::to_vec(&pointer)?)?;
        self.recheck()?;
        self.poisoned = true;
        fs::rename(&pending, self.profile.path.join("active.json"))?;
        sync_directory(&self.profile.path)?;
        // Failure from this point is an uncertain commit, never permission to
        // restart the old pair. Keep the lease until the client reconciles.
        self.poisoned = false;
        self.record("committed")
    }
    pub fn finish(&mut self, state: &str) -> Result<(), LibraryError> {
        let allowed = match state {
            "succeeded" => self.state == "committed",
            "rolled-back" => self.state == "rolling-back",
            "cancelled" | "failed" => self.state == "prepared" || self.state == "activating",
            "recovery-failed" => self.state != "committed",
            _ => false,
        };
        require(allowed, "invalid final Apply state")?;
        if state != "succeeded" {
            self.recheck()?;
        }
        self.record(state)?;
        self.poisoned = true;
        save_new(
            &self.path.join("completed.json"),
            &serde_json::to_vec(&Record {
                sequence: self.sequence - 1,
                state: state.into(),
            })?,
        )?;
        self.poisoned = false;
        Ok(())
    }
}
