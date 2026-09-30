//! Windows endpoint notifications. Callback methods do no device work and never
//! wait for the output owner. The owner retains registration through teardown.

use super::{Backend, Event, QueueConnection, Signals};
use crate::AudioError;
use rodio::cpal::traits::HostTrait;
use rodio::{OutputStream, OutputStreamHandle};
use std::sync::Arc;
use windows::core::{implement, PCWSTR};
use windows::Win32::Media::Audio::{
    eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, PKEY_AudioEngine_DeviceFormat,
    PKEY_AudioEngine_OEMFormat, DEVICE_STATE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

#[implement(IMMNotificationClient)]
struct Notifications {
    signals: Arc<Signals>,
}

impl Notifications {
    fn endpoint_changed(&self, id: &PCWSTR) {
        // Endpoint IDs originate in Windows and are copied only for equality.
        if id.is_null() {
            self.signals.notify(Event::Rescan);
        } else if let Ok(id) = unsafe { id.to_string() } {
            self.signals.notify(Event::EndpointChanged(id));
        } else {
            self.signals.notify(Event::Rescan);
        }
    }
}

#[allow(non_snake_case)]
impl IMMNotificationClient_Impl for Notifications_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            self.signals.notify(Event::DefaultChanged);
        }
        Ok(())
    }
    fn OnDeviceStateChanged(&self, id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        self.endpoint_changed(id);
        Ok(())
    }
    fn OnDeviceRemoved(&self, id: &PCWSTR) -> windows::core::Result<()> {
        self.endpoint_changed(id);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.signals.notify(Event::Rescan);
        Ok(())
    }
    fn OnPropertyValueChanged(&self, id: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        // Friendly-name and volume changes do not change the endpoint.
        if *key == PKEY_AudioEngine_DeviceFormat || *key == PKEY_AudioEngine_OEMFormat {
            self.endpoint_changed(id);
        }
        Ok(())
    }
}

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub(super) struct WindowsBackend {
    enumerator: IMMDeviceEnumerator,
    notifications: IMMNotificationClient,
    // Fields drop before COM is uninitialized on this same owner thread.
    _apartment: ComApartment,
}

impl WindowsBackend {
    pub(super) fn new(signals: Arc<Signals>) -> Result<Self, AudioError> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(native_error)?;
        let apartment = ComApartment;
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(native_error)?;
        let notifications: IMMNotificationClient = Notifications { signals }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&notifications) }
            .map_err(native_error)?;
        Ok(Self {
            enumerator,
            notifications,
            _apartment: apartment,
        })
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        if let Err(error) = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.notifications)
        } {
            if error.code().0 as u32 != 0x80070490 {
                // Registration does not AddRef. An unconfirmed unregister
                // must not leave Windows with a freed callback pointer. This
                // subscription is created once, not once per device switch.
                std::mem::forget(self.notifications.clone());
                tracing::warn!(%error, "Unregister failed; retaining the closed output callback until process exit");
            }
        }
    }
}

pub(super) struct Connection {
    _stream: OutputStream,
    _handle: OutputStreamHandle,
}

impl Backend for WindowsBackend {
    type Connection = Connection;

    fn default_device(&mut self) -> Result<Option<String>, AudioError> {
        let device = match unsafe { self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) } {
            Ok(device) => device,
            Err(error) if error.code().0 as u32 == 0x80070490 => return Ok(None), // E_NOTFOUND
            Err(error) => return Err(native_error(error)),
        };
        let id = unsafe { device.GetId() }.map_err(native_error)?;
        let copied = unsafe { id.to_string() };
        unsafe {
            CoTaskMemFree(Some(id.0.cast()));
        }
        copied.map(Some).map_err(|error| {
            AudioError::DeviceNotFound(format!("invalid Windows endpoint identity: {error}"))
        })
    }

    fn open(&mut self) -> Result<(Connection, [QueueConnection; 3]), AudioError> {
        // try_default() may silently try non-default devices. Use the selected
        // device directly; the owner verifies identity and notifications again.
        let device = rodio::cpal::default_host()
            .default_output_device()
            .ok_or_else(|| {
                AudioError::DeviceNotFound("Windows has no default playback device".into())
            })?;
        let (stream, handle) = OutputStream::try_from_device(&device).map_err(|error| {
            AudioError::DeviceNotFound(format!("Windows default output: {error}"))
        })?;
        let mut queues = Vec::with_capacity(3);
        for _ in 0..3 {
            let (queue, source) = QueueConnection::new();
            handle
                .play_raw(source)
                .map_err(|error| AudioError::PlaybackError(error.to_string()))?;
            queues.push(queue);
        }
        Ok((
            Connection {
                _stream: stream,
                _handle: handle,
            },
            queues.try_into().ok().expect("three output queues"),
        ))
    }
}

fn native_error(error: windows::core::Error) -> AudioError {
    AudioError::DeviceNotFound(format!("Windows endpoint notification API: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceRuntime, EVENT_CAPACITY};
    use crate::{AudioBuffer, PlaybackCue, PlaybackStatus, StreamType};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows::core::{w, GUID};
    use windows::Win32::Media::Audio::{eCapture, eCommunications};

    #[test]
    fn native_callback_filters_roles_and_copies_endpoint_events() {
        let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
        let signals = Arc::new(Signals {
            sender,
            revision: AtomicU64::new(0),
            overflow: AtomicBool::new(false),
            retry_requested: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let callback: IMMNotificationClient = Notifications {
            signals: signals.clone(),
        }
        .into();
        unsafe {
            callback
                .OnDefaultDeviceChanged(eCapture, eConsole, PCWSTR::null())
                .unwrap();
            callback
                .OnDefaultDeviceChanged(eRender, eCommunications, PCWSTR::null())
                .unwrap();
            callback
                .OnPropertyValueChanged(
                    w!("speakers"),
                    PROPERTYKEY {
                        fmtid: GUID::zeroed(),
                        pid: 42,
                    },
                )
                .unwrap();
        }
        assert!(receiver.try_recv().is_err());
        unsafe {
            callback
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
        }
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Event::DefaultChanged
        ));
        unsafe {
            callback.OnDeviceRemoved(w!("speakers")).unwrap();
        }
        assert!(
            matches!(receiver.try_recv().unwrap(), Event::EndpointChanged(id) if id == "speakers")
        );
        unsafe {
            callback
                .OnPropertyValueChanged(w!("headphones"), PKEY_AudioEngine_DeviceFormat)
                .unwrap();
        }
        assert!(
            matches!(receiver.try_recv().unwrap(), Event::EndpointChanged(id) if id == "headphones")
        );
        signals.closed.store(true, Ordering::Release);
        unsafe {
            callback
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    #[ignore = "requires a real Windows default output; plays only zero-valued samples"]
    fn native_output_reopens_and_retires_old_audio() {
        let (prepared, preparation) = mpsc::channel();
        let (runtime, control) = DeviceRuntime::start([8, 8, 8], move |signals| {
            let mut backend = WindowsBackend::new(signals.clone())?;
            let endpoint = backend.default_device()?.expect("default endpoint");
            prepared.send((signals, endpoint)).unwrap();
            Ok(backend)
        })
        .unwrap();
        let (signals, endpoint) = preparation.recv().unwrap();
        let generation = Arc::new(AtomicU64::new(1));
        control.bind_output_generation(generation.clone());
        let (started, observed) = mpsc::channel();
        let old = control
            .queue_tracked_with_cue_callback(
                StreamType::Speech,
                &AudioBuffer::new(vec![0.0; 44_100 * 2 * 30]),
                vec![PlaybackCue {
                    frame_offset: 0,
                    identifier: 1,
                }],
                move |_| {
                    let _ = started.send(());
                },
            )
            .unwrap()
            .unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        // Exercise the real owner and real native stream replacement without
        // changing the user's Windows default device. Physical plug/unplug is
        // a separate acceptance check.
        signals.notify(Event::EndpointChanged(endpoint.clone()));
        let (retired, retirement) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            retired.send(old.wait()).unwrap();
        });
        assert_eq!(
            retirement.recv_timeout(Duration::from_secs(5)).unwrap(),
            PlaybackStatus::Cancelled
        );
        waiter.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let fresh = loop {
            if let Ok(Some(ticket)) =
                control.queue_tracked(StreamType::Speech, &AudioBuffer::new(vec![0.0; 8820]))
            {
                break ticket;
            }
            assert!(
                Instant::now() < deadline,
                "native replacement did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let (completed, completion) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            completed.send(fresh.wait()).unwrap();
        });
        assert_eq!(
            completion.recv_timeout(Duration::from_secs(5)).unwrap(),
            PlaybackStatus::Completed
        );
        waiter.join().unwrap();
        assert!(generation.load(Ordering::Acquire) > 1);
        drop(runtime);
        assert!(control
            .queue(StreamType::Speech, &AudioBuffer::new(vec![0.0; 2]))
            .is_err());
        eprintln!("Native zero-sample output started, reopened, retired old speech and completed fresh speech on {endpoint}");
    }
}
