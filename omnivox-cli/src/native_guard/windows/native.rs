//! Exact native identities and an explicit inheritance allowlist. No PID lookup.
use std::ffi::c_void;
use std::io;
use std::os::windows::{
    ffi::OsStrExt,
    io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle, RawHandle},
};
use std::path::Path;

const SYNCHRONIZE: u32 = 0x10_0000;
#[repr(C)]
#[derive(Default)]
struct Startup {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    chars_x: u32,
    chars_y: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    input: RawHandle,
    output: RawHandle,
    error: RawHandle,
}
#[repr(C)]
struct StartupEx {
    startup: Startup,
    attributes: *mut c_void,
}
#[repr(C)]
#[derive(Default)]
struct ProcessInformation {
    process: RawHandle,
    thread: RawHandle,
    pid: u32,
    tid: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> RawHandle;
    fn DuplicateHandle(
        source: RawHandle,
        handle: RawHandle,
        target: RawHandle,
        output: *mut RawHandle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
    fn CreateEventW(
        attributes: *const c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> RawHandle;
    fn SetEvent(event: RawHandle) -> i32;
    fn WaitForSingleObject(handle: RawHandle, timeout: u32) -> u32;
    fn TerminateProcess(process: RawHandle, code: u32) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const RawHandle, all: i32, timeout: u32)
        -> u32;
    fn CreatePipe(
        read: *mut RawHandle,
        write: *mut RawHandle,
        attributes: *const c_void,
        size: u32,
    ) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_security: *const c_void,
        thread_security: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *mut StartupEx,
        information: *mut ProcessInformation,
    ) -> i32;
}

fn duplicate(
    source: RawHandle,
    handle: RawHandle,
    access: u32,
    inherit: bool,
    same: bool,
) -> io::Result<OwnedHandle> {
    let mut output = std::ptr::null_mut();
    // SAFETY: the source process is retained by callers; Windows validates the
    // remote handle. A successful copy transfers into this process exactly once.
    if unsafe {
        DuplicateHandle(
            source,
            handle,
            GetCurrentProcess(),
            &mut output,
            access,
            i32::from(inherit),
            if same { 2 } else { 0 },
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful DuplicateHandle returned a fresh owned local handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(output) })
}

pub(crate) fn current_process() -> io::Result<OwnedHandle> {
    // SAFETY: GetCurrentProcess returns the documented current-process pseudo handle.
    let current = unsafe { GetCurrentProcess() };
    duplicate(current, current, SYNCHRONIZE | 1, false, false)
}
pub(crate) fn signalled(handle: BorrowedHandle<'_>) -> io::Result<bool> {
    // SAFETY: retained waitable object; a zero timeout does not block.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        0 => Ok(true),
        258 => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}
pub(crate) struct Event(OwnedHandle);
impl Event {
    pub fn new() -> io::Result<Self> {
        Self::create(true)
    }
    pub fn auto() -> io::Result<Self> {
        Self::create(false)
    }
    pub fn handle(&self) -> BorrowedHandle<'_> {
        self.0.as_handle()
    }
    fn create(manual: bool) -> io::Result<Self> {
        // SAFETY: unnamed, noninheritable, manual-reset event, initially false.
        let raw = unsafe { CreateEventW(std::ptr::null(), i32::from(manual), 0, std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fresh successful event creation transfers ownership once.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
    pub fn signal(&self) -> io::Result<()> {
        // SAFETY: this event retains EVENT_MODIFY_STATE permission.
        if unsafe { SetEvent(self.0.as_raw_handle()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn observer(&self) -> io::Result<OwnedHandle> {
        // SAFETY: documented process pseudo handle, live source event.
        duplicate(
            unsafe { GetCurrentProcess() },
            self.0.as_raw_handle(),
            SYNCHRONIZE,
            false,
            false,
        )
    }
}

pub(crate) struct Process {
    _handle: OwnedHandle,
}
#[cfg(test)]
impl Process {
    pub(crate) fn handle(&self) -> BorrowedHandle<'_> {
        self._handle.as_handle()
    }
}

struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: created by InitializeProcThreadAttributeList below; backing
        // allocation and inherited copies outlive CreateProcessW.
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

/// Six explicit handles and a NUL standard stream are the complete allowlist.
/// No job handle is inherited. Speech endpoints leave the guard at worker spawn.
pub(crate) fn spawn(executable: &Path, handles: [BorrowedHandle<'_>; 6]) -> io::Result<Process> {
    if !executable.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let path: Vec<u16> = executable.as_os_str().encode_wide().collect();
    if path.contains(&0) || path.contains(&(b'"' as u16)) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut copies = Vec::with_capacity(6);
    for handle in handles {
        // SAFETY: documented current-process pseudo handle, live borrowed source.
        copies.push(duplicate(
            unsafe { GetCurrentProcess() },
            handle.as_raw_handle(),
            0,
            true,
            true,
        )?);
    }
    let mut raw: Vec<_> = copies.iter().map(AsRawHandle::as_raw_handle).collect();
    let arguments = raw
        .iter()
        .map(|h| (*h as usize).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")?;
    // SAFETY: live NUL file and documented pseudo handle. Its inheritable copy
    // is included explicitly and closes when process creation returns.
    let null = duplicate(
        unsafe { GetCurrentProcess() },
        null.as_raw_handle(),
        0,
        true,
        true,
    )?;
    raw.push(null.as_raw_handle());
    let suffix = format!(" {} {arguments}", crate::native_guard::MODE);
    let mut command = vec![b'"' as u16];
    command.extend_from_slice(&path);
    command.push(b'"' as u16);
    command.extend(suffix.encode_utf16());
    command.push(0);
    let mut application = path;
    application.push(0);
    let environment = std::ptr::null();
    let mut size = 0;
    // SAFETY: documented sizing call, no output buffer provided.
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
    }
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0_usize; size.div_ceil(size_of::<usize>())];
    // SAFETY: suitably aligned allocation of at least the requested size.
    if unsafe { InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut size) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = Attributes(storage);
    // SAFETY: PROC_THREAD_ATTRIBUTE_HANDLE_LIST, live inheritable handles, exact
    // byte size, buffer remains alive until after CreateProcessW returns.
    if unsafe {
        UpdateProcThreadAttribute(
            attributes.0.as_mut_ptr().cast(),
            0,
            0x20002,
            raw.as_ptr().cast(),
            size_of_val(raw.as_slice()),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut startup = StartupEx {
        startup: Startup {
            size: size_of::<StartupEx>() as u32,
            flags: 0x100, // STARTF_USESTDHANDLES
            input: null.as_raw_handle(),
            output: null.as_raw_handle(),
            error: null.as_raw_handle(),
            ..Default::default()
        },
        attributes: attributes.0.as_mut_ptr().cast(),
    };
    let mut information = ProcessInformation::default();
    // SAFETY: fully initialized structures and terminated UTF-16 strings.
    // EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            0x0808_0400,
            environment,
            std::ptr::null(),
            &mut startup,
            &mut information,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful process creation returns these two distinct handles.
    let handle = unsafe { OwnedHandle::from_raw_handle(information.process) };
    drop(unsafe { OwnedHandle::from_raw_handle(information.thread) });
    Ok(Process { _handle: handle })
}

/// Consume private inherited handles once, clear inheritance before any threads
/// or workers are created. Validation copies first, so malformed argv never
/// constructs an OwnedHandle from an unchecked integer.
pub(crate) fn receive(arguments: &[String]) -> io::Result<[OwnedHandle; 6]> {
    if arguments.len() != 6 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let values: Vec<u64> = arguments
        .iter()
        .map(|v| v.parse().map_err(|_| io::ErrorKind::InvalidInput))
        .collect::<Result<_, _>>()?;
    let mut unique = values.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != 6 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut result = Vec::new();
    for value in values {
        if value == 0 || value >= isize::MAX as u64 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let raw = value as usize as RawHandle;
        // SAFETY: Windows validates the inherited value before any ownership
        // conversion. Copies are noninheritable; originals close below.
        let copy = duplicate(unsafe { GetCurrentProcess() }, raw, 0, false, true)?;
        // SAFETY: a successful duplicate established a live handle; private
        // entry receives exclusive ownership of these distinct inherited values.
        let original = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: live original. Clear inheritance even if a later argument fails.
        if unsafe { SetHandleInformation(original.as_raw_handle(), 1, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        result.push(copy);
    }
    result
        .try_into()
        .map_err(|_| io::ErrorKind::InvalidInput.into())
}

pub(crate) fn terminate(handle: BorrowedHandle<'_>) -> io::Result<()> {
    if signalled(handle)? {
        return Ok(());
    }
    // SAFETY: actual retained owner identity with PROCESS_TERMINATE, never a PID lookup.
    if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } == 0 && !signalled(handle)? {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub(crate) fn wait(handles: &[RawHandle], timeout: u32) -> io::Result<()> {
    if handles.is_empty() || handles.len() > 64 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // SAFETY: all handles are borrowed from live owners on the waiting thread.
    let result =
        unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, timeout) };
    if result == u32::MAX {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub(crate) fn speech_pipe() -> io::Result<(std::fs::File, std::fs::File)> {
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    // SAFETY: valid output storage; NULL security makes both handles noninheritable.
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful creation transfers each distinct pipe handle exactly once.
    Ok(unsafe {
        (
            std::fs::File::from_raw_handle(read),
            std::fs::File::from_raw_handle(write),
        )
    })
}
