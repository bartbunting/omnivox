//! Narrow Win32 ABI for owned local overlapped pipes; no native dependencies.
use std::ffi::c_void;
use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};

pub type Handle = *mut c_void;
pub const OVERLAPPED: u32 = 0x4000_0000;
pub const FIRST_INSTANCE: u32 = 0x0008_0000;
pub const REJECT_REMOTE: u32 = 8;
pub const PENDING: i32 = 997;
pub const INCOMPLETE: i32 = 996;
pub const NOT_FOUND: i32 = 1168;
pub const PIPE_CONNECTED: i32 = 535;
pub const PIPE_BUSY: i32 = 231;
pub const BROKEN_PIPE: i32 = 109;
pub const PIPE_NOT_CONNECTED: i32 = 233;

#[repr(C)]
#[derive(Default)]
pub struct Overlapped {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: [u32; 2],
    pub event: Handle,
}
#[repr(C)]
pub struct SecurityAttributes {
    pub length: u32,
    pub descriptor: *mut c_void,
    pub inherit: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        output: u32,
        input: u32,
        timeout: u32,
        attributes: *const SecurityAttributes,
    ) -> Handle;
    pub fn ConnectNamedPipe(pipe: Handle, operation: *mut Overlapped) -> i32;
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const SecurityAttributes,
        creation: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    pub fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        count: u32,
        transferred: *mut u32,
        operation: *mut Overlapped,
    ) -> i32;
    pub fn WriteFile(
        file: Handle,
        buffer: *const u8,
        count: u32,
        transferred: *mut u32,
        operation: *mut Overlapped,
    ) -> i32;
    pub fn GetOverlappedResult(
        file: Handle,
        operation: *mut Overlapped,
        transferred: *mut u32,
        wait: i32,
    ) -> i32;
    pub fn CancelIoEx(file: Handle, operation: *mut Overlapped) -> i32;
    pub fn CreateEventW(
        attributes: *const c_void,
        manual_reset: i32,
        initial: i32,
        name: *const u16,
    ) -> Handle;
    pub fn ResetEvent(event: Handle) -> i32;
    pub fn GetCurrentProcess() -> Handle;
    pub fn LocalFree(memory: *mut c_void) -> *mut c_void;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    pub fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    pub fn GetTokenInformation(
        token: Handle,
        class: u32,
        buffer: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    pub fn ConvertSidToStringSidW(sid: *const c_void, text: *mut *mut u16) -> i32;
    pub fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        length: *mut u32,
    ) -> i32;
    pub fn GetSecurityInfo(
        handle: Handle,
        object_type: u32,
        information: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        descriptor: *mut *mut c_void,
    ) -> u32;
}

/// Take a newly created kernel handle, which can use either failure sentinel.
pub unsafe fn owned(handle: Handle) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle as isize == -1 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: the caller transfers the sole new owning handle.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
