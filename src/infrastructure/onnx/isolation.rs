//! Process containment for user-supplied ONNX workloads.

use super::model::run_trusted_user_onnx_with_options;
use super::{OnnxInferenceOptions, OnnxInputLayout, OnnxNormalization};
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{run_command, CommandSpec, TemporaryOutput};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const ONNX_WALL_TIMEOUT: Duration = Duration::from_secs(30);
const ONNX_WORKER_MEMORY_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_WORKER_REPORT_BYTES: u64 = 1024 * 1024;
const SAMPLER_FAILURE_REAP_GRACE: Duration = Duration::from_millis(50);
const SAMPLER_FAILURE_REAP_INTERVAL: Duration = Duration::from_millis(5);
pub(crate) const ONNX_WORKER_PROTOCOL: &str = concat!(env!("CARGO_PKG_VERSION"), ":1");

pub(crate) fn run_isolated_onnx(
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
) -> Result<super::OnnxReport> {
    validate_onnx_paths(input, model)?;
    let _ = options;
    Err(default_worker_unavailable(
        "automatic worker discovery is disabled because a current-executable-derived path cannot authenticate the worker image; use run_user_onnx_with_worker with an explicit worker path",
    ))
}

fn validate_onnx_paths(input: &Path, model: &Path) -> Result<()> {
    if !input.is_file() {
        return Err(ScanError::Invalid(format!(
            "ONNX input image not found: {}",
            input.display()
        )));
    }
    if !model.is_file() {
        return Err(ScanError::Invalid(format!(
            "ONNX model not found: {}",
            model.display()
        )));
    }
    Ok(())
}

fn default_worker_unavailable(reason: impl std::fmt::Display) -> ScanError {
    ScanError::Unsupported(format!(
        "no trusted Open Scanline ONNX worker is available: {reason}"
    ))
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

pub(crate) fn run_isolated_onnx_with_executable(
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
    executable: &Path,
) -> Result<super::OnnxReport> {
    validate_onnx_paths(input, model)?;
    let executable = executable.canonicalize().map_err(|error| {
        ScanError::Invalid(format!(
            "ONNX worker executable not found: {} ({error})",
            executable.display()
        ))
    })?;
    if !executable.is_file() {
        return Err(ScanError::Invalid(format!(
            "ONNX worker executable not found: {}",
            executable.display()
        )));
    }
    let _trusted_identity = TrustedWorkerHandle::acquire(&executable, "ONNX worker executable")?;
    validate_worker_executable(&executable)?;
    let report = TemporaryOutput::new("onnx-report", "json")?;
    let mut args = vec![
        "__onnx-worker".to_string(),
        "--in".to_string(),
        input.to_string_lossy().into_owned(),
        "--model".to_string(),
        model.to_string_lossy().into_owned(),
        "--report".to_string(),
        report.path().to_string_lossy().into_owned(),
        "--layout".to_string(),
        layout_arg(options.layout).to_string(),
        "--normalization".to_string(),
        normalization_arg(options.normalization).to_string(),
        "--worker-protocol".to_string(),
        ONNX_WORKER_PROTOCOL.to_string(),
    ];
    if let Some(input_name) = options.input_name.as_deref() {
        args.push("--input-name".to_string());
        args.push(input_name.to_string());
    }
    let status = supervise_worker(&executable, &args, ONNX_WALL_TIMEOUT)?;
    if !status.success() {
        return Err(ScanError::Other(format!(
            "ONNX worker exited with status {status}"
        )));
    }
    let metadata = std::fs::metadata(report.path())
        .map_err(|error| ScanError::Other(format!("ONNX worker produced no report: {error}")))?;
    if metadata.len() > MAX_WORKER_REPORT_BYTES {
        return Err(ScanError::Other(
            "ONNX worker report exceeded its size limit".into(),
        ));
    }
    let bytes = std::fs::read(report.path())?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ScanError::Other(format!("invalid ONNX worker report: {error}")))
}

#[cfg(not(windows))]
struct TrustedWorkerHandle;

#[cfg(not(windows))]
impl TrustedWorkerHandle {
    fn acquire(_path: &Path, _role: &str) -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
struct TrustedWorkerHandle {
    _file: std::fs::File,
}

#[cfg(windows)]
fn has_exactly_one_hard_link(file: &std::fs::File) -> std::io::Result<bool> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileInformation {
        file_attributes: u32,
        creation_time_low: u32,
        creation_time_high: u32,
        last_access_time_low: u32,
        last_access_time_high: u32,
        last_write_time_low: u32,
        last_write_time_high: u32,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut c_void) -> i32;
    }

    let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
    // SAFETY: `file` is an open Windows file handle and `information` points to writable
    // storage with the exact layout required by GetFileInformationByHandle.
    let success = unsafe {
        GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr().cast())
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a successful GetFileInformationByHandle call initializes all fields.
    Ok(unsafe { information.assume_init() }.number_of_links == 1)
}

#[cfg(windows)]
impl TrustedWorkerHandle {
    fn acquire(path: &Path, role: &str) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| {
                default_worker_unavailable(format!(
                    "could not retain {role} {} without write/delete sharing: {error}",
                    path.display()
                ))
            })?;
        let metadata = file.metadata().map_err(|error| {
            default_worker_unavailable(format!(
                "could not inspect retained {role} {}: {error}",
                path.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_file() || metadata.len() == 0 {
            return Err(default_worker_unavailable(format!(
                "retained {role} {} is empty, non-regular, or a reparse point",
                path.display()
            )));
        }

        if !has_exactly_one_hard_link(&file).map_err(|error| {
            default_worker_unavailable(format!(
                "could not inspect retained {role} {} link count: {error}",
                path.display()
            ))
        })? {
            return Err(default_worker_unavailable(format!(
                "retained {role} {} must have exactly one hard link",
                path.display()
            )));
        }
        Ok(Self { _file: file })
    }
}

fn validate_worker_executable(executable: &Path) -> Result<()> {
    let expected = format!("open-scanline {}", env!("CARGO_PKG_VERSION"));
    let cancelled = std::sync::Mutex::new(false);
    let output = run_command(
        &CommandSpec {
            program: executable.to_string_lossy().into_owned(),
            args: vec!["--version".into()],
        },
        Duration::from_secs(5),
        &cancelled,
        "ONNX worker version probe",
        "ONNX worker version probe cancelled",
    )?;
    let reported = std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .unwrap_or_default();
    if !output.success || reported != expected {
        return Err(ScanError::Unsupported(format!(
            "ONNX worker executable is not the version-matched Open Scanline worker (expected '{expected}')"
        )));
    }
    Ok(())
}

pub(crate) fn run_onnx_worker(
    input: &Path,
    model: &Path,
    report: &Path,
    options: &OnnxInferenceOptions,
    worker_protocol: &str,
) -> Result<()> {
    if worker_protocol != ONNX_WORKER_PROTOCOL {
        return Err(ScanError::Unsupported(format!(
            "ONNX worker protocol mismatch: expected {ONNX_WORKER_PROTOCOL}"
        )));
    }
    start_parent_liveness_watchdog();
    apply_worker_limits()?;
    set_worker_thread_limits();
    let image = crate::infrastructure::media::load_image(input)?;
    let result = run_trusted_user_onnx_with_options(&image, model, options)?;
    let json = serde_json::to_vec(&result)
        .map_err(|error| ScanError::Other(format!("could not serialize ONNX report: {error}")))?;
    if json.len() as u64 > MAX_WORKER_REPORT_BYTES {
        return Err(ScanError::Other(
            "ONNX worker report exceeded its size limit".into(),
        ));
    }
    crate::infrastructure::runtime::atomic_publish::write_file_atomic(report, &json)
}

/// The supervisor keeps the worker's standard-input pipe open for its entire
/// lifetime. If the parent crashes or is interrupted, EOF wakes this thread
/// and terminates the otherwise orphaned model process on every host OS.
fn start_parent_liveness_watchdog() {
    std::thread::spawn(|| {
        use std::io::Read;

        let mut input = std::io::stdin().lock();
        let mut byte = [0_u8; 1];
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => std::process::exit(1),
                Ok(_) => {}
            }
        }
    });
}

fn supervise_worker(
    executable: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::ExitStatus> {
    supervise_worker_with_memory_limit(executable, args, timeout, ONNX_WORKER_MEMORY_LIMIT_BYTES)
}

fn supervise_worker_with_memory_limit(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    memory_limit: u64,
) -> Result<std::process::ExitStatus> {
    #[cfg(target_os = "macos")]
    let sampler = |pid| worker_resident_bytes(pid).map(Some);
    #[cfg(not(target_os = "macos"))]
    let sampler = |_| Ok(None);
    supervise_worker_with_memory_limit_and_sampler(executable, args, timeout, memory_limit, sampler)
}

fn supervise_worker_with_memory_limit_and_sampler<F>(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    memory_limit: u64,
    mut sampler: F,
) -> Result<std::process::ExitStatus>
where
    F: FnMut(u32) -> std::io::Result<Option<u64>>,
{
    let working_directory = worker_working_directory(executable)?;
    let mut child = Command::new(executable)
        .args(args)
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ScanError::Other(format!("ONNX worker failed to start: {error}")))?;
    // Retaining this write end is the worker's parent-liveness signal.
    let _liveness = child.stdin.take();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScanError::Other(format!(
                    "ONNX worker exceeded the {}-second wall-time limit",
                    timeout.as_secs()
                )));
            }
            Ok(None) => {
                match sampler(child.id()) {
                    Ok(Some(resident)) if resident > memory_limit => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(ScanError::Unsupported(format!(
                            "ONNX worker exceeded the {} MiB resident-memory limit",
                            memory_limit / (1024 * 1024)
                        )));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if let Some(status) = reap_natural_exit_after_sampler_failure(&mut child) {
                            return Ok(status);
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(ScanError::Unsupported(format!(
                            "ONNX worker containment is unavailable: could not sample resident memory: {error}"
                        )));
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScanError::Other(format!(
                    "ONNX worker supervision failed: {error}"
                )));
            }
        }
    }
}

fn worker_working_directory(executable: &Path) -> Result<&Path> {
    if !executable.is_absolute() {
        return Err(ScanError::Invalid(format!(
            "ONNX worker executable must be absolute: {}",
            executable.display()
        )));
    }
    executable.parent().ok_or_else(|| {
        ScanError::Invalid(format!(
            "ONNX worker executable has no parent directory: {}",
            executable.display()
        ))
    })
}

/// A process can exit after `try_wait` but before a macOS RSS sample. Give that
/// normal exit a short chance to become reapable before treating the sampler
/// failure as a containment failure. Any still-live or indeterminate process
/// remains fail-closed: the caller kills and reaps it.
fn reap_natural_exit_after_sampler_failure(
    child: &mut std::process::Child,
) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + SAMPLER_FAILURE_REAP_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if std::time::Instant::now() >= deadline => return None,
            Ok(None) => std::thread::sleep(SAMPLER_FAILURE_REAP_INTERVAL),
            Err(_) => return None,
        }
    }
}

#[cfg(target_os = "macos")]
fn worker_resident_bytes(pid: u32) -> std::io::Result<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let expected = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            expected,
        )
    };
    if read == expected {
        Ok(unsafe { info.assume_init().pti_resident_size })
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn layout_arg(layout: OnnxInputLayout) -> &'static str {
    match layout {
        OnnxInputLayout::Auto => "auto",
        OnnxInputLayout::Nchw => "nchw",
        OnnxInputLayout::Nhwc => "nhwc",
    }
}

fn normalization_arg(normalization: OnnxNormalization) -> &'static str {
    match normalization {
        OnnxNormalization::ZeroToOne => "zero-to-one",
        OnnxNormalization::None => "none",
    }
}

fn set_worker_thread_limits() {
    for (name, value) in [
        ("RAYON_NUM_THREADS", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("OPENBLAS_NUM_THREADS", "1"),
        ("MKL_NUM_THREADS", "1"),
    ] {
        std::env::set_var(name, value);
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn apply_worker_limits() -> Result<()> {
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
    let status = unsafe { libc::setrlimit(resource, &limit) };
    if status == 0 {
        Ok(())
    } else {
        Err(ScanError::Unsupported(format!(
            "could not apply ONNX worker {name} limit: {}",
            std::io::Error::last_os_error()
        )))
    }
}

// Darwin exposes RLIMIT_AS but rejects ordinary process attempts to lower it
// on supported desktop releases. The supervised parent deadline, bounded IPC,
// and static graph/tensor limits remain effective; hard RSS enforcement is a
// documented best-effort platform limitation.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn apply_worker_limits() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn apply_worker_limits() -> Result<()> {
    windows_job::apply()
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
fn apply_worker_limits() -> Result<()> {
    Err(ScanError::Unsupported(
        "ONNX worker containment is unavailable on this platform".into(),
    ))
}

#[cfg(windows)]
mod windows_job {
    use crate::error::{Result, ScanError};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr;

    type Handle = *mut c_void;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_PROCESS_TIME: u32 = 0x0000_0002;
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const PROCESS_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;
    const PROCESS_TIME_100NS: i64 = 20 * 10_000_000;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
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
        let mut limits = ExtendedLimitInformation::default();
        limits.basic_limit_information.per_process_user_time_limit = PROCESS_TIME_100NS;
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_PROCESS_TIME
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits.process_memory_limit = PROCESS_MEMORY_BYTES;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                (&limits as *const ExtendedLimitInformation).cast(),
                size_of::<ExtendedLimitInformation>() as u32,
            )
        };
        let assigned =
            configured != 0 && unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } != 0;
        if !assigned {
            unsafe {
                CloseHandle(job);
            }
            return Err(last_error("contain ONNX worker process"));
        }
        // The job handle intentionally remains open for the worker lifetime.
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
