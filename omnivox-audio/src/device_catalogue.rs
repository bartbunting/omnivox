//! Bounded output discovery without speech-engine construction.

use crate::AudioError;
use omnivox_core::audio_device::DeviceCatalogue;

/// Enumerate supported physical device choices on this speech host.
pub fn output_devices() -> Result<DeviceCatalogue, AudioError> {
    #[cfg(windows)]
    {
        use omnivox_core::audio_device::{
            DeviceSelection, OutputDevice, MAX_DEVICES, MAX_DEVICE_NAME_BYTES,
        };
        use rodio::cpal::traits::{DeviceTrait, HostTrait};
        let host = rodio::cpal::default_host();
        let default_device = host
            .default_output_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let mut devices = Vec::new();
        let mut truncated = false;
        for device in host
            .output_devices()
            .map_err(|error| AudioError::DeviceNotFound(error.to_string()))?
        {
            if devices.len() == MAX_DEVICES {
                truncated = true;
                break;
            }
            let id = device
                .id()
                .map_err(|error| AudioError::DeviceNotFound(error.to_string()))?
                .to_string();
            DeviceSelection::parse(&id).map_err(|error| AudioError::InvalidFormat(error.into()))?;
            let description = device
                .description()
                .map_err(|error| AudioError::DeviceNotFound(error.to_string()))?;
            let mut name = description.name().to_owned();
            if name.len() > MAX_DEVICE_NAME_BYTES {
                let mut boundary = MAX_DEVICE_NAME_BYTES;
                while !name.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                name.truncate(boundary);
            }
            devices.push(OutputDevice { id, name });
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(DeviceCatalogue {
            host_scope: output_device_host_scope()?,
            backend: "device".into(),
            supported: true,
            default_device,
            devices,
            truncated,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(DeviceCatalogue {
            host_scope: std::env::consts::OS.into(),
            backend: "device".into(),
            supported: false,
            default_device: None,
            devices: Vec::new(),
            truncated: false,
        })
    }
}

pub fn output_device_host_scope() -> Result<String, AudioError> {
    #[cfg(not(windows))]
    {
        Ok(std::env::consts::OS.into())
    }
    #[cfg(windows)]
    {
        windows_host_scope()
    }
}

#[cfg(windows)]
fn windows_host_scope() -> Result<String, AudioError> {
    use windows::Win32::System::SystemInformation::{
        ComputerNamePhysicalDnsHostname, GetComputerNameExW,
    };
    let mut buffer = [0_u16; 256];
    let mut size = buffer.len() as u32;
    unsafe {
        GetComputerNameExW(
            ComputerNamePhysicalDnsHostname,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .map_err(|error| AudioError::DeviceNotFound(format!("speech host identity: {error}")))?;
    let name = String::from_utf16(&buffer[..size as usize])
        .map_err(|_| AudioError::InvalidFormat("invalid speech host name".into()))?;
    Ok(format!("windows:wasapi:{}", name.to_lowercase()))
}
