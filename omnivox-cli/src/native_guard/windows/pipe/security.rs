//! A protected DACL grants access only to the creating process's Windows user.
use super::native::*;
use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr::{null, null_mut};

pub(super) struct Local(pub *mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: owned allocation returned by a Win32 conversion function.
        unsafe { LocalFree(self.0) };
    }
}

pub(super) fn user_sid() -> io::Result<String> {
    let mut token = null_mut();
    // SAFETY: valid output and process pseudo-handle; TOKEN_QUERY only.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) })?;
    let token = unsafe { owned(token)? };
    // TOKEN_USER plus the maximum SID fits in this aligned, bounded allocation.
    let mut user = [0usize; 128];
    let mut length = 0;
    // SAFETY: writable aligned storage, TokenUser class 1, live token handle.
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            1,
            user.as_mut_ptr().cast(),
            std::mem::size_of_val(&user) as u32,
            &mut length,
        )
    })?;
    let sid = user[0] as *const c_void; // TOKEN_USER starts with SID_AND_ATTRIBUTES.
    sid_string(sid)
}

fn sid_string(sid: *const c_void) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: callers retain the native descriptor/token buffer owning this SID.
    check(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    let _text = Local(text.cast());
    let mut count = 0;
    // SAFETY: the returned SID string is terminated and at most 184 UTF-16 units.
    while count < 184 && unsafe { *text.add(count) } != 0 {
        count += 1;
    }
    if count == 184 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(text, count) })
        .map_err(|_| io::ErrorKind::InvalidData.into())
}

pub(super) struct Security(Vec<usize>);
impl Security {
    pub fn current_user() -> io::Result<Self> {
        let sid = user_sid()?;
        let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
        let mut descriptor = null_mut();
        let mut length = 0;
        // SAFETY: terminated SDDL and writable outputs; SDDL_REVISION_1.
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                &mut length,
            )
        })?;
        let _descriptor = Local(descriptor);
        // The returned descriptor is self-relative: copying preserves offsets.
        // Retain native alignment as well as the complete returned bytes.
        let mut copy = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            std::ptr::copy_nonoverlapping(
                descriptor.cast::<u8>(),
                copy.as_mut_ptr().cast(),
                length as usize,
            )
        };
        Ok(Self(copy))
    }

    pub fn pipe(&self, name: &[u16], first: bool) -> io::Result<std::os::windows::io::OwnedHandle> {
        self.create(
            name,
            3 | OVERLAPPED | if first { FIRST_INSTANCE } else { 0 },
            super::MAX_INSTANCES,
            super::BUFFER as u32,
        )
    }

    pub fn helper_pipe(
        &self,
        name: &[u16],
        readable: bool,
    ) -> io::Result<std::os::windows::io::OwnedHandle> {
        self.create(
            name,
            (if readable { 1 } else { 2 }) | OVERLAPPED | FIRST_INSTANCE,
            1,
            // Match the pinned Rust standard child pipe capacity. Control
            // framing/buffers remain smaller and independent of helper PCM.
            64 * 1024,
        )
    }

    fn create(
        &self,
        name: &[u16],
        mode: u32,
        instances: u32,
        capacity: u32,
    ) -> io::Result<std::os::windows::io::OwnedHandle> {
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor: self.0.as_ptr().cast_mut().cast(),
            inherit: 0,
        };
        // SAFETY: valid name and self-relative descriptor; a non-inheritable,
        // local-only byte pipe with overlapped I/O. Keep the instance
        // bound below the Win32 unlimited sentinel (255).
        unsafe {
            owned(CreateNamedPipeW(
                name.as_ptr(),
                mode,
                REJECT_REMOTE,
                instances,
                capacity,
                capacity,
                0,
                &attributes,
            ))
        }
    }
}

pub(super) fn client(name: &[u16]) -> io::Result<std::os::windows::io::OwnedHandle> {
    connect(name, 0xc000_0000, OVERLAPPED)
}

pub(super) fn helper_client(
    name: &[u16],
    worker_reads: bool,
) -> io::Result<std::os::windows::io::OwnedHandle> {
    connect(
        name,
        if worker_reads {
            0x4000_0000
        } else {
            0x8000_0000
        },
        0,
    )
}

fn connect(name: &[u16], access: u32, flags: u32) -> io::Result<std::os::windows::io::OwnedHandle> {
    // SAFETY: validated local name, no inheritance, OPEN_EXISTING. Explicit
    // SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION prevents impersonation by
    // the server. No WaitNamedPipe or filesystem/network fallback is used.
    let handle = unsafe {
        owned(CreateFileW(
            name.as_ptr(),
            access,
            0,
            null(),
            3,
            flags | 0x0011_0000,
            null_mut(),
        ))?
    };
    // Inspect the actual opened object, not a pathname or a saved process ID.
    let mut owner = null_mut();
    let mut descriptor = null_mut();
    let result = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            1,
            1,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result as i32));
    }
    let _descriptor = Local(descriptor);
    if owner.is_null() || sid_string(owner)? != user_sid()? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(handle)
}
