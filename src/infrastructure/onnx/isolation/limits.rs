use crate::error::Result;
#[cfg(any(target_os = "linux", target_os = "android", windows))]
use crate::error::ScanError;

pub(super) fn set_worker_thread_limits() {
    for (name, value) in [
        ("RAYON_NUM_THREADS", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("OPENBLAS_NUM_THREADS", "1"),
        ("MKL_NUM_THREADS", "1"),
    ] {
        unsafe { std::env::set_var(name, value) };
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn apply_worker_limits() -> Result<()> {
    const ADDRESS_SPACE_BYTES: libc::rlim_t = 2 * 1024 * 1024 * 1024;
    const CPU_SECONDS: libc::rlim_t = 20;
    set_limit(libc::RLIMIT_AS, ADDRESS_SPACE_BYTES, "address space")?;
    set_limit(libc::RLIMIT_CPU, CPU_SECONDS, "CPU time")
}
#[cfg(any(target_os = "linux", target_os = "android"))]
type RlimitResource = libc::__rlimit_resource_t;
#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_limit(resource: RlimitResource, value: libc::rlim_t, name: &str) -> Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(ScanError::Unsupported(format!(
            "could not apply ONNX worker {name} limit: {}",
            std::io::Error::last_os_error()
        )))
    }
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(super) fn apply_worker_limits() -> Result<()> {
    Ok(())
}
#[cfg(windows)]
pub(super) fn apply_worker_limits() -> Result<()> {
    windows_job::apply()
}
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
pub(super) fn apply_worker_limits() -> Result<()> {
    Err(ScanError::Unsupported(
        "ONNX worker containment is unavailable on this platform".into(),
    ))
}

#[cfg(windows)]
mod windows_job {
    use super::*;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr;
    type Handle = *mut c_void;
    const CLASS: i32 = 9;
    const PROCESS_TIME: u32 = 0x2;
    const PROCESS_MEMORY: u32 = 0x100;
    const KILL_ON_CLOSE: u32 = 0x2000;
    const MEMORY: usize = 2 * 1024 * 1024 * 1024;
    const TIME: i64 = 20 * 10_000_000;
    #[repr(C)]
    #[derive(Default)]
    struct Basic {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min: usize,
        max: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Io {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read: u64,
        write: u64,
        other: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Limits {
        basic: Basic,
        io: Io,
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: i32,
            information: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }
    pub(super) fn apply() -> Result<()> {
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(last_error("create ONNX worker job"));
        }
        let mut limits = Limits::default();
        limits.basic.process_time = TIME;
        limits.basic.flags = PROCESS_TIME | PROCESS_MEMORY | KILL_ON_CLOSE;
        limits.process_memory = MEMORY;
        let assigned = unsafe {
            SetInformationJobObject(
                job,
                CLASS,
                (&limits as *const Limits).cast(),
                size_of::<Limits>() as u32,
            )
        } != 0
            && unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } != 0;
        if !assigned {
            unsafe {
                CloseHandle(job);
            }
            return Err(last_error("contain ONNX worker process"));
        }
        std::mem::forget(JobHandle(job));
        Ok(())
    }
    struct JobHandle(Handle);
    impl Drop for JobHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn last_error(action: &str) -> ScanError {
        ScanError::Unsupported(format!(
            "could not {action}: {}",
            std::io::Error::last_os_error()
        ))
    }
}
