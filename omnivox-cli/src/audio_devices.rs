//! Bounded local output management. Native work and replies stay off admission.

use anyhow::Result;
use omnivox_audio::AudioControl;
use omnivox_core::audio_device::DeviceOutcome;
use omnivox_tts::audio_devices::{DeviceResult, DeviceStatus};
use omnivox_tts::control::{
    decode_request, ControlErrorCode, ControlRequest, ControlResponse, ControlResponseEnvelope,
    CONTROL_PROTOCOL_VERSION,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

pub(crate) fn available(control: &AudioControl) -> bool {
    control.audio_device_state().is_some()
        && std::env::var_os("OMNIVOX_REMOTE_WORKER").is_none()
        && !crate::emergency::worker()
}

pub(crate) struct Runtime {
    control: Arc<AudioControl>,
    incarnation: String,
    host_scope: String,
    reports: mpsc::SyncSender<ControlResponseEnvelope>,
    discovery: mpsc::SyncSender<u64>,
    closed: Arc<AtomicBool>,
}

impl Runtime {
    pub(crate) fn start(control: Arc<AudioControl>) -> Result<Option<Self>> {
        if !available(&control) {
            return Ok(None);
        }
        let incarnation = omnivox_tts::voice_library::local::new_uuid()?;
        let host_scope = omnivox_audio::output_device_host_scope()?;
        let closed = Arc::new(AtomicBool::new(false));
        let (reports, receiver) = mpsc::sync_channel(16);
        let stopped = closed.clone();
        std::thread::Builder::new()
            .name("omnivox-output-replies".into())
            .spawn(move || {
                while let Ok(response) = receiver.recv() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    crate::server::write_control_response(&response);
                }
            })?;
        let (discovery, requests) = mpsc::sync_channel(1);
        let stopped = closed.clone();
        let output = reports.clone();
        std::thread::Builder::new()
            .name("omnivox-output-discovery".into())
            .spawn(move || {
                while let Ok(request_id) = requests.recv() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let response = match omnivox_audio::output_devices() {
                        Ok(devices) => ControlResponse::AudioDevicesV1(devices),
                        Err(error) => {
                            tracing::warn!(%error, "Audio-device discovery failed");
                            ControlResponse::Error {
                                code: ControlErrorCode::InvalidConfiguration,
                                message: "Audio-device discovery failed; refresh to retry".into(),
                            }
                        }
                    };
                    if !stopped.load(Ordering::Acquire) {
                        let _ = output.try_send(envelope(request_id, response));
                    }
                }
            })?;
        Ok(Some(Self {
            control,
            incarnation,
            host_scope,
            reports,
            discovery,
            closed,
        }))
    }

    fn status(&self) -> DeviceStatus {
        DeviceStatus {
            incarnation: self.incarnation.clone(),
            host_scope: self.host_scope.clone(),
            state: self
                .control
                .audio_device_state()
                .expect("attached output manager"),
        }
    }

    pub(crate) fn try_handle(&self, payload: &str) -> bool {
        let Ok(request) = decode_request(payload) else {
            return false;
        };
        if request.protocol_version != CONTROL_PROTOCOL_VERSION {
            return false;
        }
        let response = match request.request {
            ControlRequest::AudioDevicesV1 => {
                if self.discovery.try_send(request.request_id).is_ok() {
                    return true;
                }
                ControlResponse::Error {
                    code: ControlErrorCode::InvalidConfiguration,
                    message: "Audio-device discovery is busy; refresh to retry".into(),
                }
            }
            ControlRequest::AudioDeviceStatusV1 => {
                ControlResponse::AudioDeviceStatusV1(self.status())
            }
            ControlRequest::ApplyAudioDeviceV1(operation) => {
                let sequence = operation.change.sequence;
                let outcome = if operation.incarnation != self.incarnation {
                    DeviceOutcome::Stale
                } else {
                    self.control
                        .begin_audio_device_change(operation.change)
                        .unwrap_or(DeviceOutcome::Rejected)
                };
                let status = self.status();
                let outcome = if outcome == DeviceOutcome::Preparing {
                    status
                        .state
                        .last_operation
                        .as_ref()
                        .filter(|op| op.change.sequence == sequence)
                        .map_or(outcome, |op| op.outcome)
                } else {
                    outcome
                };
                ControlResponse::AudioDeviceResultV1(DeviceResult {
                    sequence,
                    outcome,
                    status,
                })
            }
            ControlRequest::CancelAudioDeviceV1(operation) => {
                let outcome = if operation.incarnation != self.incarnation {
                    DeviceOutcome::Stale
                } else {
                    self.control
                        .cancel_audio_device_change(operation.sequence)
                        .unwrap_or(DeviceOutcome::Rejected)
                };
                ControlResponse::AudioDeviceResultV1(DeviceResult {
                    sequence: operation.sequence,
                    outcome,
                    status: self.status(),
                })
            }
            _ => return false,
        };
        // Losing a reply does not undo an output change. The client queries its
        // incarnation/sequence again; a blocked stdout cannot hold Stop here.
        let _ = self
            .reports
            .try_send(envelope(request.request_id, response));
        true
    }
}

fn envelope(request_id: u64, response: ControlResponse) -> ControlResponseEnvelope {
    ControlResponseEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        request_id: Some(request_id),
        response,
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}
