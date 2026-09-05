//! Windows job ownership keeps remote workers and helpers tied to the broker.
use anyhow::{Context, Result};
use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct ExtendedLimits {
    basic: BasicLimits,
    io_counters: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, data: *const c_void, size: u32) -> i32;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn TerminateJobObject(job: Handle, status: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

pub struct Job(Handle);

impl Job {
    pub fn new() -> Result<Self> {
        // SAFETY: null attributes/name create an unnamed, non-inheritable job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error()).context("could not create worker job");
        }
        let job = Self(handle);
        let mut limits = ExtendedLimits::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                                     // SAFETY: repr(C) matches JOBOBJECT_EXTENDED_LIMIT_INFORMATION; class 9.
        if unsafe {
            SetInformationJobObject(
                job.0,
                9,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<ExtendedLimits>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("could not configure worker job");
        }
        Ok(job)
    }

    pub fn assign(&self, child: &Child) -> Result<()> {
        // SAFETY: both handles are live and owned by this process.
        if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("could not own worker process tree");
        }
        Ok(())
    }

    pub fn terminate(&self) {
        // SAFETY: only processes assigned to our private job are affected.
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: this is the sole owning handle; descendants die with the job.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
