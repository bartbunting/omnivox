//! Suspended creation into an already-owned job. No process-ID authority.
use super::*;
use std::ffi::OsStr;
use std::process::Command;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn ResumeThread(thread: RawHandle) -> u32;
    fn CompareStringOrdinal(a: *const u16, na: i32, b: *const u16, nb: i32, ignore: i32) -> i32;
    fn GetFullPathNameW(path: *const u16, size: u32, output: *mut u16, part: *mut *mut u16) -> u32;
}

pub(crate) const WORKER_MODE: &str = "--native-helper-worker";

pub(crate) struct Created {
    pub process: OwnedHandle,
    thread: Option<OwnedHandle>,
    stopping: bool,
}
impl Created {
    pub fn release(&mut self) -> io::Result<()> {
        let thread = self
            .thread
            .as_ref()
            .ok_or_else(|| io::Error::other("native gate already released"))?;
        // SAFETY: this is the actual retained primary thread created suspended.
        if unsafe { ResumeThread(thread.as_raw_handle()) } != 1 {
            return Err(io::Error::other(
                "native gate did not have exactly one suspension",
            ));
        }
        self.thread.take();
        Ok(())
    }
    pub fn ended(&self) -> io::Result<bool> {
        signalled(self.process.as_handle())
    }
    pub fn terminate(&mut self) -> io::Result<()> {
        if self.stopping || self.ended()? {
            return Ok(());
        }
        match terminate(self.process.as_handle()) {
            Ok(()) => (),
            // CreateProcess supplied full termination authority. Another owner
            // may already have initiated exit; ACCESS_DENIED is not completion.
            // Keep waiting on the actual process object in either case.
            Err(error) if error.raw_os_error() == Some(5) => (),
            Err(error) => return Err(error),
        }
        self.stopping = true;
        Ok(())
    }
}

/// Duplicate only into the actual retained receiving process. A returned number
/// belongs to that process, never this one. Lost delivery requires its retirement.
pub(crate) fn transfer(
    source: BorrowedHandle<'_>,
    target: BorrowedHandle<'_>,
    access: Option<u32>,
) -> io::Result<usize> {
    let mut output = std::ptr::null_mut();
    // SAFETY: both source and target are live owned objects. The target is never
    // reopened from a PID. Non-inheritable copies cannot leak into helpers.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source.as_raw_handle(),
            target.as_raw_handle(),
            &mut output,
            access.unwrap_or(0),
            0,
            if access.is_none() { 2 } else { 0 },
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(output as usize)
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut text: Vec<_> = value.encode_wide().collect();
    if text.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    text.push(0);
    Ok(text)
}

// Like Rust's ordinary Command, give CreateProcess a non-verbatim application
// name when Win32 resolves it identically. .NET Framework otherwise rejects its
// own configuration path. Keep the retained identity and argv unchanged. This
// is a bounded lexical OS check, not executable discovery or another disk scan.
fn application_path(program: &Path) -> io::Result<Vec<u16>> {
    use std::path::{Component, Prefix};
    let original = wide(program.as_os_str())?;
    if original.len() > 260 {
        return Ok(original);
    }
    let ordinary = match program.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::VerbatimDisk(_) => original[4..].to_vec(),
            Prefix::VerbatimUNC(_, _) => {
                let mut path = vec![b'\\' as u16; 2];
                path.extend_from_slice(&original[8..]);
                path
            }
            _ => return Ok(original),
        },
        _ => return Ok(original),
    };
    let mut normalized = vec![0; 260];
    // SAFETY: NUL-terminated input and a live output allocation of the supplied
    // size. No returned file-part pointer is requested or retained.
    let length = unsafe {
        GetFullPathNameW(
            ordinary.as_ptr(),
            normalized.len() as u32,
            normalized.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    } as usize;
    if length > 0
        && length < normalized.len()
        && normalized[..length] == ordinary[..ordinary.len() - 1]
    {
        normalized.truncate(length + 1);
        Ok(normalized)
    } else {
        Ok(original)
    }
}

// CRT argument quoting, including empty arguments and trailing backslashes.
fn argument(value: &OsStr, output: &mut Vec<u16>) -> io::Result<()> {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        output.push(unit);
        slashes = 0;
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(())
}

fn environment(command: &Command) -> io::Result<Vec<u16>> {
    let mut entries = Vec::new();
    for (key, value) in command.get_envs() {
        let Some(value) = value else { continue };
        let mut key = wide(key)?;
        key.pop();
        if key.is_empty() || key.len() > 32767 || key.contains(&(b'=' as u16)) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let value = wide(value)?;
        entries.push((key, value));
    }
    entries.sort_by(|(a, _), (b, _)| {
        // SAFETY: complete live UTF-16 slices, including native unpaired units.
        // Snapshot bounds fit i32. CSTR_LESS/EQUAL/GREATER are 1/2/3.
        let order = unsafe {
            CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1)
        };
        order.cmp(&2)
    });
    let mut output = Vec::new();
    for (key, value) in entries {
        output.extend(key);
        output.push(b'=' as u16);
        output.extend(value);
    }
    output.push(0);
    if output.len() == 1 {
        output.push(0);
    }
    Ok(output)
}

/// COMMAND must contain the complete retained environment. It is not inherited
/// implicitly. The worker-only extra channel is passed ahead of ordinary argv.
pub(crate) fn spawn(
    command: &Command,
    job: BorrowedHandle<'_>,
    stdio: [BorrowedHandle<'_>; 3],
    worker_channel: Option<BorrowedHandle<'_>>,
) -> io::Result<Created> {
    let program = Path::new(command.get_program());
    if !program.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let application = application_path(program)?;
    let directory = command
        .get_current_dir()
        .map(|p| wide(p.as_os_str()))
        .transpose()?;
    let environment = environment(command)?;
    let mut copies = Vec::with_capacity(4);
    for handle in stdio.into_iter().chain(worker_channel) {
        // SAFETY: every source remains borrowed for the duration of CreateProcess.
        copies.push(duplicate(
            unsafe { GetCurrentProcess() },
            handle.as_raw_handle(),
            0,
            true,
            true,
        )?);
    }
    let handles: Vec<_> = copies.iter().map(AsRawHandle::as_raw_handle).collect();
    let mut args = Vec::new();
    argument(program.as_os_str(), &mut args)?;
    if let Some(channel) = handles.get(3) {
        args.push(b' ' as u16);
        argument(OsStr::new(WORKER_MODE), &mut args)?;
        args.push(b' ' as u16);
        argument(OsStr::new(&(*channel as usize).to_string()), &mut args)?;
    }
    for value in command.get_args() {
        args.push(b' ' as u16);
        argument(value, &mut args)?;
    }
    args.push(0);
    if args.len() > 32767 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut bytes = 0;
    // SAFETY: documented sizing call followed by a suitably aligned allocation.
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 3, 0, &mut bytes);
    }
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    if unsafe { InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 3, 0, &mut bytes) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attrs = Attributes(buffer);
    let job = job.as_raw_handle();
    let restricted = 1_u32;
    for (kind, value, size) in [
        (
            0x20002,
            handles.as_ptr().cast(),
            size_of_val(handles.as_slice()),
        ),
        (
            0x2000d,
            (&job as *const RawHandle).cast(),
            size_of::<RawHandle>(),
        ),
        (
            0x2000e,
            (&restricted as *const u32).cast(),
            size_of::<u32>(),
        ),
    ] {
        // SAFETY: HANDLE_LIST, JOB_LIST and CHILD_PROCESS_POLICY point into live
        // exact-sized values retained through creation. Job ownership is atomic.
        if unsafe {
            UpdateProcThreadAttribute(
                attrs.0.as_mut_ptr().cast(),
                0,
                kind,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let mut startup = StartupEx {
        startup: Startup {
            size: size_of::<StartupEx>() as u32,
            flags: 0x100,
            input: handles[0],
            output: handles[1],
            error: handles[2],
            ..Default::default()
        },
        attributes: attrs.0.as_mut_ptr().cast(),
    };
    let mut info = ProcessInformation::default();
    // SAFETY: explicit inherited handles, native terminated strings, complete
    // Unicode environment. SUSPENDED | DETACHED_PROCESS | EXTENDED | UNICODE.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            args.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            0x0008_040c,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            &mut startup,
            &mut info,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful creation returns two fresh distinct owned handles.
    Ok(Created {
        process: unsafe { OwnedHandle::from_raw_handle(info.process) },
        thread: Some(unsafe { OwnedHandle::from_raw_handle(info.thread) }),
        stopping: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_names_preserve_verbatim_semantics_and_native_units() {
        for (input, expected) in [
            (r"\\?\C:\speech\helper.exe", r"C:\speech\helper.exe"),
            (
                r"\\?\UNC\server\share\helper.exe",
                r"\\server\share\helper.exe",
            ),
            (
                r"\\?\C:\speech\..\helper.exe",
                r"\\?\C:\speech\..\helper.exe",
            ),
            (r"\\?\C:\speech\helper.exe.", r"\\?\C:\speech\helper.exe."),
        ] {
            assert_eq!(
                application_path(Path::new(input)).unwrap(),
                wide(OsStr::new(expected)).unwrap()
            );
        }
        use std::os::windows::ffi::OsStringExt;
        let mut path: Vec<_> = r"\\?\C:\speech\".encode_utf16().collect();
        path.push(0xd800);
        path.extend(".exe".encode_utf16());
        let native = std::ffi::OsString::from_wide(&path);
        let mut expected = path[4..].to_vec();
        expected.push(0);
        assert_eq!(application_path(Path::new(&native)).unwrap(), expected);
        let long = format!(r"\\?\C:\{}\helper.exe", "x".repeat(260));
        assert_eq!(
            application_path(Path::new(&long)).unwrap(),
            wide(OsStr::new(&long)).unwrap()
        );
    }
    #[test]
    fn quoted_native_arguments_keep_empty_quotes_and_trailing_slashes() {
        for (source, expected) in [
            ("", "\"\""),
            ("a b", "\"a b\""),
            ("a\\", "\"a\\\\\""),
            ("a\"b", "\"a\\\"b\""),
        ] {
            let mut output = Vec::new();
            argument(OsStr::new(source), &mut output).unwrap();
            assert_eq!(String::from_utf16(&output).unwrap(), expected);
        }
        assert!(argument(OsStr::new("a\0b"), &mut Vec::new()).is_err());
    }
}
