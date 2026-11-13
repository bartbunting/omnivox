//! CoreAudio output ownership. System listeners wake the owner; CPAL reports
//! invalidated connections. Native setup and teardown stay on the owner thread.

use super::notifications::{self, Registration};
use super::{Backend, Event, QueueConnection, Signals};
use crate::AudioError;
use objc2_core_audio::{
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    AudioObjectAddPropertyListener, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListener,
};
use omnivox_core::audio_device::DeviceSelection;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::cpal::{Device, DeviceId, ErrorKind, HostId};
use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

struct Listener {
    address: AudioObjectPropertyAddress,
    registration: Option<Registration>,
}

impl Listener {
    fn new(signals: &Arc<Signals>, default_changed: bool) -> Result<Self, AudioError> {
        let registration = Registration::new(signals, default_changed)?;
        let address = AudioObjectPropertyAddress {
            mSelector: if default_changed {
                kAudioHardwarePropertyDefaultOutputDevice
            } else {
                kAudioHardwarePropertyDevices
            },
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        // SAFETY: the address lives through the call. CoreAudio treats client
        // data as an opaque cookie; our callback never dereferences it. The
        // registry keeps only a Weak reference and rejects retired cookies.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                Some(changed),
                std::ptr::without_provenance_mut(registration.0),
            )
        };
        if status != 0 {
            return Err(AudioError::DeviceNotFound(format!(
                "CoreAudio notification registration: OSStatus {status}"
            )));
        }
        Ok(Self {
            address,
            registration: Some(registration),
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let registration = self.registration.take().unwrap();
        let cookie = registration.0;
        // Retire the lookup before native unregister, so delayed callbacks can
        // never access freed context, even if CoreAudio rejects unregister.
        drop(registration);
        // SAFETY: same object, address, function and opaque cookie as Add.
        let status = unsafe {
            AudioObjectRemovePropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&self.address),
                Some(changed),
                std::ptr::without_provenance_mut(cookie),
            )
        };
        if status != 0 {
            tracing::warn!(
                status,
                "CoreAudio listener unregister failed; callback is inert"
            );
        }
    }
}

unsafe extern "C-unwind" fn changed(
    _object: AudioObjectID,
    _count: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    cookie: *mut c_void,
) -> i32 {
    notifications::notify(cookie.addr());
    0
}

pub(super) struct MacBackend {
    _listeners: [Listener; 2],
    signals: Arc<Signals>,
}

impl MacBackend {
    pub(super) fn new(signals: Arc<Signals>) -> Result<Self, AudioError> {
        Ok(Self {
            _listeners: [
                Listener::new(&signals, true)?,
                Listener::new(&signals, false)?,
            ],
            signals,
        })
    }
}

impl Backend for MacBackend {
    type Connection = MixerDeviceSink;

    fn default_device(&mut self) -> Result<Option<String>, AudioError> {
        rodio::cpal::default_host()
            .default_output_device()
            .map(|device| device.id().map(|id| id.to_string()).map_err(device_error))
            .transpose()
    }

    fn resolve(&mut self, selection: &DeviceSelection) -> Result<Option<String>, AudioError> {
        let id = match selection {
            DeviceSelection::SystemDefault => self.default_device()?,
            DeviceSelection::Device { id } => Some(id.clone()),
        };
        match id {
            Some(id) => Ok(find_device(&id)?.map(|_| id)),
            None => Ok(None),
        }
    }

    fn open(&mut self, id: &str) -> Result<(MixerDeviceSink, [QueueConnection; 3]), AudioError> {
        // Always use an enumerated device, even for follow-default. CPAL's
        // default_output_device creates an auto-routing AudioUnit; enumeration
        // creates a pinned HAL output. Only our owner may publish a new route.
        let device = find_device(id)?.ok_or_else(|| {
            AudioError::DeviceNotFound("selected CoreAudio output is unavailable".into())
        })?;
        let signals = self.signals.clone();
        let id = id.to_owned();
        let mut stream = DeviceSinkBuilder::from_device(device)
            .map_err(|error| AudioError::DeviceNotFound(error.to_string()))?
            .with_error_callback(move |error| {
                // Underruns and scheduling warnings do not invalidate a stream.
                // CPAL delivers disconnect/rate changes away from the IO thread.
                if invalidates_output(error.kind()) {
                    signals.notify(Event::EndpointChanged(id.clone()));
                }
            })
            // Rodio's fallback here tries formats on this exact device only.
            .open_sink_or_fallback()
            .map_err(|error| AudioError::DeviceNotFound(format!("CoreAudio output: {error}")))?;
        stream.log_on_drop(false);
        let queues = std::array::from_fn(|_| {
            let (queue, source) = QueueConnection::new();
            stream.mixer().add(source);
            queue
        });
        Ok((stream, queues))
    }
}

fn invalidates_output(kind: ErrorKind) -> bool {
    !matches!(kind, ErrorKind::Xrun | ErrorKind::RealtimeDenied)
}

fn find_device(id: &str) -> Result<Option<Device>, AudioError> {
    let parsed = id
        .parse::<DeviceId>()
        .map_err(|error| AudioError::InvalidFormat(error.to_string()))?;
    if parsed.host() != HostId::CoreAudio {
        return Err(AudioError::InvalidFormat(
            "device belongs to a different audio backend".into(),
        ));
    }
    // Preserve enumeration failures as errors. Search by UID, never by the
    // transient AudioObjectID or friendly label. Output filtering rejects inputs.
    for device in rodio::cpal::default_host()
        .output_devices()
        .map_err(device_error)?
    {
        if device.id().map_err(device_error)? == parsed {
            return Ok(Some(device));
        }
    }
    Ok(None)
}

fn device_error(error: rodio::cpal::Error) -> AudioError {
    AudioError::DeviceNotFound(format!("CoreAudio device: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceRuntime;
    use omnivox_core::audio_device::{DeviceChange, DeviceOutcome};
    use std::time::{Duration, Instant};

    #[test]
    fn unrelated_backend_is_rejected_before_native_discovery() {
        assert!(find_device("wasapi:headphones").is_err());
    }

    #[test]
    fn transient_audio_warnings_do_not_reopen_the_device() {
        assert!(!invalidates_output(ErrorKind::Xrun));
        assert!(!invalidates_output(ErrorKind::RealtimeDenied));
        assert!(invalidates_output(ErrorKind::DeviceNotAvailable));
        assert!(invalidates_output(ErrorKind::StreamInvalidated));
        assert!(invalidates_output(ErrorKind::HostUnavailable));
    }

    #[test]
    #[ignore = "requires a native Mac with an available CoreAudio output"]
    fn native_default_explicit_default_preserves_the_output_owner() {
        let catalogue = crate::output_devices().unwrap();
        assert!(catalogue.supported);
        assert!(catalogue.host_scope.starts_with("macos:coreaudio:"));
        let id = catalogue.default_device.unwrap();
        assert!(catalogue.devices.iter().any(|device| device.id == id));
        let (_runtime, control) =
            DeviceRuntime::new([8; 3], DeviceSelection::SystemDefault).unwrap();
        let initial = control.audio_device_state().unwrap();
        assert_eq!(initial.connected.as_deref(), Some(id.as_str()));
        for (sequence, selection) in [
            DeviceSelection::Device { id: id.clone() },
            DeviceSelection::SystemDefault,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                control.begin_audio_device_change(DeviceChange {
                    sequence: sequence as u64 + 1,
                    expected_revision: sequence as u64,
                    selection: selection.clone(),
                }),
                Some(DeviceOutcome::Preparing)
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let state = control.audio_device_state().unwrap();
                let operation = state.last_operation.unwrap();
                if operation.outcome != DeviceOutcome::Preparing {
                    assert_eq!(operation.outcome, DeviceOutcome::Applied, "{operation:?}");
                    assert_eq!(state.selection, selection);
                    assert_eq!(state.connected.as_deref(), Some(id.as_str()));
                    break;
                }
                assert!(Instant::now() < deadline, "CoreAudio Apply timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
