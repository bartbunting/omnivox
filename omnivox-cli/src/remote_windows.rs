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
    fn QueryInformationJobObject(
        job: Handle,
        class: i32,
        data: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

pub struct Job(Handle);

impl Job {
    pub fn new() -> Result<Self> {
        Self::with_memory_limit(None)
    }

    pub fn for_validation(memory: usize) -> Result<Self> {
        Self::with_memory_limit(Some(memory))
    }

    fn with_memory_limit(memory: Option<usize>) -> Result<Self> {
        // SAFETY: null attributes/name create an unnamed, non-inheritable job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error()).context("could not create worker job");
        }
        let job = Self(handle);
        let mut limits = ExtendedLimits::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if let Some(memory) = memory {
            limits.basic.flags |= 0x200; // JOB_OBJECT_LIMIT_JOB_MEMORY
            limits.job_memory = memory;
        }
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

    pub fn terminate_checked(&self) -> std::io::Result<()> {
        // SAFETY: this handle owns only the private validation tree.
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn is_empty(&self) -> std::io::Result<bool> {
        #[repr(C)]
        #[derive(Default)]
        struct Accounting {
            user_time: i64,
            kernel_time: i64,
            period_user_time: i64,
            period_kernel_time: i64,
            page_faults: u32,
            total_processes: u32,
            active_processes: u32,
            terminated_processes: u32,
        }
        let mut accounting = Accounting::default();
        // SAFETY: class 1 uses JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, represented
        // by the repr(C) structure above; the handle and output pointer are live.
        if unsafe {
            QueryInformationJobObject(
                self.0,
                1,
                &mut accounting as *mut _ as *mut c_void,
                std::mem::size_of::<Accounting>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(accounting.active_processes == 0)
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
