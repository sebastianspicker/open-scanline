//! Shared process and image-decoding seams for command-backed scanner adapters.

use crate::core::{
    checked_image_len, ImageBuffer, Result, ScanError, ScanRequest, MAX_IMAGE_BYTES,
};
use crate::device::{CancellationToken, DeviceInfo};
use std::fs;
use std::io::Read;
use std::path::Path;
#[cfg(not(windows))]
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver},
    Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ARTIFACT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bound a document-feeder process while allowing the deadline to scale with
/// the requested logical side count. A large feeder job can legitimately run
/// for hours at high DPI, but it must never become unbounded.
pub(crate) fn document_batch_timeout(max_pages: u32) -> Duration {
    const BASE_SECONDS: u64 = 60;
    const SECONDS_PER_SIDE: u64 = 45;
    const MAX_SECONDS: u64 = 12 * 60 * 60;
    Duration::from_secs(
        BASE_SECONDS
            .saturating_add(u64::from(max_pages).saturating_mul(SECONDS_PER_SIDE))
            .min(MAX_SECONDS),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The bounded amount of on-disk output a native acquisition is allowed to
/// materialize in one private artifact directory.
///
/// This is deliberately separate from stdout/stderr capture. Scanner drivers
/// often write images directly to files, so pipe limits do not bound their
/// disk use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactQuota {
    pub max_files: u64,
    pub max_bytes: u64,
}

/// A private artifact directory and the quota enforced while a child runs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArtifactWatch<'a> {
    pub(crate) directory: &'a Path,
    pub(crate) quota: ArtifactQuota,
}

#[derive(Debug, Clone, Copy)]
struct CommandOutputLimits<'a> {
    capture_bytes: usize,
    artifacts: Option<ArtifactWatch<'a>>,
}

const ARTIFACT_PER_IMAGE_OVERHEAD: u64 = 1024 * 1024;

/// Derive a checked, request-specific native artifact budget.
///
/// Each requested logical side receives enough room for its decoded pixel
/// buffer plus a small encoded-image overhead. The whole directory remains
/// capped near the product-wide decoded-image ceiling, so a feeder cannot use
/// its page limit to turn a bounded scan into an unbounded disk write.
pub(crate) fn artifact_quota_for_request(
    request: &ScanRequest,
    max_pages: u32,
) -> Result<ArtifactQuota> {
    let (width, height) = request
        .region
        .map(|region| (region.width.max(1), region.height.max(1)))
        .unwrap_or((request.width.max(1), request.height.max(1)));
    let image_bytes = u64::try_from(checked_image_len(
        width,
        height,
        request.pixel_format.bpp(),
    )?)
    .map_err(|_| ScanError::Invalid("image size does not fit the artifact quota".into()))?;
    let per_image = image_bytes
        .checked_add(ARTIFACT_PER_IMAGE_OVERHEAD)
        .ok_or_else(|| ScanError::Invalid("artifact quota overflow".into()))?;
    let requested_bytes = per_image
        .checked_mul(u64::from(max_pages))
        .ok_or_else(|| ScanError::Invalid("artifact quota overflow".into()))?;
    let global_limit = u64::try_from(MAX_IMAGE_BYTES)
        .expect("usize image limit always fits in u64")
        .checked_add(ARTIFACT_PER_IMAGE_OVERHEAD)
        .expect("fixed artifact overhead fits in u64");
    Ok(ArtifactQuota {
        max_files: u64::from(max_pages),
        max_bytes: requested_bytes.min(global_limit),
    })
}

pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
    ) -> Result<CommandOutput>;

    /// Token-aware extension point. Existing injected runners only need to
    /// implement [`CommandRunner::run`]; production overrides this to interrupt
    /// a child blocked inside native acquisition.
    fn run_with_cancellation(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<CommandOutput> {
        let _ = cancellation;
        self.run(spec, timeout, cancelled)
    }

    /// Artifact-aware extension point. The default keeps injected test and
    /// integration runners source-compatible; native runners override it to
    /// supervise direct-to-file scanner output while the command is running.
    fn run_with_cancellation_and_artifact_quota(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
        artifact_directory: &Path,
        artifact_quota: ArtifactQuota,
    ) -> Result<CommandOutput> {
        let _ = (artifact_directory, artifact_quota);
        self.run_with_cancellation(spec, timeout, cancelled, cancellation)
    }
}

pub trait ImageDecoder: Send + Sync {
    fn decode(&self, path: &Path) -> Result<ImageBuffer>;
}

pub struct NativeImageDecoder;

impl ImageDecoder for NativeImageDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        crate::imaging::load_image(path)
    }
}

/// An isolated, per-acquisition output directory that is removed on every
/// return path. The directory is created atomically, so concurrent scans never
/// share an output path; on Unix its contents are private to the current user.
pub(crate) struct TemporaryOutput {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl TemporaryOutput {
    pub(crate) fn new(adapter: &str, extension: &str) -> Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let sequence = ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                ".open-scanline-{adapter}-{}-{timestamp}-{sequence}",
                std::process::id(),
            ));
            match create_private_directory(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        path: directory.join(format!("output.{extension}")),
                        directory,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate unique scanner output directory",
        )
        .into())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Reject an artifact directory that has exceeded its file or byte budget.
/// Native SANE and WIA output is deliberately flat; nested directories are
/// rejected instead of traversed, keeping quota checks bounded even for a
/// malicious or broken driver. Symlinks and other non-regular entries are
/// rejected rather than followed into caller-controlled locations.
pub(crate) fn validate_artifact_quota(directory: &Path, quota: ArtifactQuota) -> Result<()> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;

    let entries = fs::read_dir(directory).map_err(|error| {
        ScanError::Unsupported(format!(
            "could not inspect native artifact directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ScanError::Unsupported(format!(
                "could not inspect native artifact directory {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            ScanError::Unsupported(format!(
                "could not inspect native artifact {}: {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_dir() {
            return Err(ScanError::Unsupported(format!(
                "native artifact output contains unexpected nested directory {}",
                path.display()
            )));
        }
        if !metadata.file_type().is_file() {
            return Err(ScanError::Unsupported(format!(
                "native artifact output contains unexpected non-regular entry {}",
                path.display()
            )));
        }
        files = files.saturating_add(1);
        bytes = bytes.saturating_add(metadata.len());
        if files > quota.max_files || bytes > quota.max_bytes {
            return Err(ScanError::Unsupported(format!(
                "native artifact output exceeded the {} file / {} byte quota",
                quota.max_files, quota.max_bytes
            )));
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    fs::DirBuilder::new().create(path)
}

/// Shared lifecycle flags for command-backed device sessions.
///
/// Reads retain the adapters' poison recovery behavior, while writes preserve
/// their best-effort semantics when a prior holder poisoned a lock.
#[derive(Default)]
pub(crate) struct CommandSession {
    closed: Mutex<bool>,
    cancelled: Mutex<bool>,
    cancellation: Mutex<Option<CancellationToken>>,
}

impl CommandSession {
    pub(crate) fn is_closed(&self) -> bool {
        *self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self
            .cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            || self
                .cancellation
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    pub(crate) fn cancelled(&self) -> &Mutex<bool> {
        &self.cancelled
    }

    pub(crate) fn cancel(&self) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            *cancelled = true;
        }
        if let Ok(cancellation) = self.cancellation.lock() {
            if let Some(token) = cancellation.as_ref() {
                token.cancel();
            }
        }
    }

    pub(crate) fn bind_cancellation(&self, token: CancellationToken) {
        if let Ok(mut cancellation) = self.cancellation.lock() {
            *cancellation = Some(token);
        }
    }

    pub(crate) fn cancellation_token(&self) -> Option<CancellationToken> {
        self.cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            *closed = true;
        }
    }

    pub(crate) fn decode_materialized_output(
        &self,
        output: &CommandOutput,
        output_path: &Path,
        decoder: &dyn ImageDecoder,
        request: &crate::core::ScanRequest,
        failure_name: &str,
    ) -> Result<ImageBuffer> {
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if !output.success || !output_path.is_file() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(ScanError::Unsupported(format!(
                "{failure_name}: {}",
                error.trim()
            )));
        }
        let mut image = decoder.decode(output_path)?;
        let (target_width, target_height) = request
            .region
            .map(|region| (region.width, region.height))
            .unwrap_or((request.width, request.height));
        if target_width > 0
            && target_height > 0
            && (image.width != target_width || image.height != target_height)
        {
            image = crate::device::FileDeviceSession::resize_nearest(
                &image,
                target_width,
                target_height,
            )?;
        }
        Ok(image)
    }
}

pub(crate) fn run_command(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_contained_command(
        spec,
        timeout,
        cancelled,
        None,
        command_name,
        cancellation_message,
    )
}

const COMMAND_STREAM_CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
const AVAILABILITY_PROBE_CAPTURE_LIMIT: usize = 64 * 1024;
const COMMAND_STREAM_DRAIN_GRACE: Duration = Duration::from_millis(250);

/// Run a short, low-output command used only to determine whether a backend is
/// usable. This deliberately shares the process-tree supervisor with scans so
/// a wedged driver probe cannot stall backend discovery.
pub(crate) fn availability_probe_succeeds(
    spec: &CommandSpec,
    timeout: Duration,
    command_name: &str,
) -> bool {
    let cancelled = Mutex::new(false);
    run_command_with_capture_limit(
        spec,
        timeout,
        &cancelled,
        None,
        command_name,
        "availability probe cancelled",
        CommandOutputLimits {
            capture_bytes: AVAILABILITY_PROBE_CAPTURE_LIMIT,
            artifacts: None,
        },
    )
    .is_ok_and(|output| output.success)
}

pub(crate) fn run_command_with_cancellation(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_contained_command(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
    )
}

/// Run a contained command with bounded pipes, cancellation, and process-tree
/// cleanup, but no direct-to-file artifact monitoring.
pub(crate) fn run_contained_command(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_command_with_capture_limit(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        CommandOutputLimits {
            capture_bytes: COMMAND_STREAM_CAPTURE_LIMIT,
            artifacts: None,
        },
    )
}

/// Run a contained command while repeatedly checking its private artifact
/// directory. A quota breach terminates and reaps the whole contained tree
/// before returning, so callers never decode untrusted over-limit output.
pub(crate) fn run_contained_command_with_artifact_quota(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
    artifact_watch: ArtifactWatch<'_>,
) -> Result<CommandOutput> {
    run_command_with_capture_limit(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        CommandOutputLimits {
            capture_bytes: COMMAND_STREAM_CAPTURE_LIMIT,
            artifacts: Some(artifact_watch),
        },
    )
}

fn run_command_with_capture_limit(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
    output_limits: CommandOutputLimits<'_>,
) -> Result<CommandOutput> {
    let (child, containment) = spawn_contained_command(spec).map_err(|error| {
        ScanError::Unsupported(format!("{command_name} failed to start: {error}"))
    })?;
    supervise_command_child(
        Supervision {
            child,
            containment,
            timeout,
            cancelled,
            cancellation,
            command_name,
            cancellation_message,
            capture_limit: output_limits.capture_bytes,
            artifact_watch: output_limits
                .artifacts
                .map(|watch| (watch.directory, watch.quota)),
        },
        |child| child.poll(),
    )
}

#[cfg(not(windows))]
fn spawn_contained_command(
    spec: &CommandSpec,
) -> std::io::Result<(PlatformChild, ProcessContainment)> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_containment(&mut command);
    let mut child = command.spawn()?;
    let containment = match ProcessContainment::for_child(&child) {
        Ok(containment) => containment,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    Ok((child, containment))
}

#[cfg(unix)]
type PlatformChild = std::process::Child;

#[cfg(not(any(unix, windows)))]
type PlatformChild = std::process::Child;

/// Unix children start in their own process group. Killing that group reaches
/// ordinary descendants before the direct child is reaped. A descendant that
/// deliberately calls `setsid` or otherwise changes process groups escapes
/// this containment boundary by design.
#[cfg(unix)]
fn configure_process_containment(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Keep the command tree out of the parent's terminal process group. The
    // standard API can use posix_spawn on supported platforms; a custom
    // pre_exec hook forces fork and can run GUI-framework atfork handlers that
    // restore SIGINT's default disposition in an all-features CLI process.
    command.process_group(0);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_containment(_: &mut Command) {}

struct ProcessContainment {
    #[cfg(unix)]
    process_group: libc::pid_t,
    #[cfg(windows)]
    job: WindowsJob,
}

impl ProcessContainment {
    #[cfg(not(windows))]
    fn for_child(child: &PlatformChild) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                process_group: child.id() as libc::pid_t,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            Ok(Self {})
        }
    }

    #[cfg(windows)]
    fn from_job(job: WindowsJob) -> Self {
        Self { job }
    }

    fn terminate<C: SupervisedChild>(&self, child: &mut C) {
        #[cfg(unix)]
        {
            // A negative PID targets the dedicated process group. Do this
            // before waiting so descendants cannot retain command pipes.
            let _ = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
        }
        #[cfg(windows)]
        {
            let _ = self.job.terminate();
        }
        let _ = child.kill();
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl WindowsJob {
    fn create() -> std::io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Self { handle };
        let mut limits = JobObjectExtendedLimitInformation::kill_on_close();
        let configured = unsafe {
            SetInformationJobObject(
                job.handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                (&mut limits as *mut JobObjectExtendedLimitInformation).cast(),
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };
        if configured == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, process: *mut std::ffi::c_void) -> std::io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn terminate(&self) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE is the backstop for every error and unwind path.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
#[cfg(windows)]
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
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

#[cfg(windows)]
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

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[cfg(windows)]
impl JobObjectExtendedLimitInformation {
    fn kill_on_close() -> Self {
        let mut limits = Self::default();
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(
        job_attributes: *const std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn SetInformationJobObject(
        job: *mut std::ffi::c_void,
        information_class: u32,
        information: *const std::ffi::c_void,
        information_length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn CreatePipe(
        read_pipe: *mut *mut std::ffi::c_void,
        write_pipe: *mut *mut std::ffi::c_void,
        attributes: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const std::ffi::c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    fn ResumeThread(thread: *mut std::ffi::c_void) -> u32;
    fn TerminateProcess(process: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: *mut std::ffi::c_void, exit_code: *mut u32) -> i32;
    fn InitializeProcThreadAttributeList(
        attribute_list: *mut std::ffi::c_void,
        attribute_count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        attribute_list: *mut std::ffi::c_void,
        flags: u32,
        attribute: usize,
        value: *mut std::ffi::c_void,
        size: usize,
        previous_value: *mut std::ffi::c_void,
        return_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(attribute_list: *mut std::ffi::c_void);
}

#[cfg(windows)]
const HANDLE_FLAG_INHERIT: u32 = 1;
#[cfg(windows)]
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;
#[cfg(windows)]
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
#[cfg(windows)]
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
#[cfg(windows)]
const WAIT_OBJECT_0: u32 = 0;
#[cfg(windows)]
const WAIT_TIMEOUT: u32 = 258;
#[cfg(windows)]
const INFINITE: u32 = u32::MAX;
#[cfg(windows)]
const INVALID_DWORD: u32 = u32::MAX;

#[cfg(windows)]
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut std::ffi::c_void,
    inherit_handle: i32,
}

#[cfg(windows)]
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_count: u16,
    reserved2: *mut u8,
    std_input: *mut std::ffi::c_void,
    std_output: *mut std::ffi::c_void,
    std_error: *mut std::ffi::c_void,
}

/// Exact `STARTUPINFOEXW` layout: a `STARTUPINFOW` prefix followed by the
/// optional attribute-list pointer used to whitelist inherited handles.
#[cfg(windows)]
#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    attribute_list: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl Default for StartupInfoExW {
    fn default() -> Self {
        Self {
            startup_info: StartupInfoW::default(),
            attribute_list: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl Default for StartupInfoW {
    fn default() -> Self {
        Self {
            cb: std::mem::size_of::<Self>() as u32,
            reserved: std::ptr::null_mut(),
            desktop: std::ptr::null_mut(),
            title: std::ptr::null_mut(),
            x: 0,
            y: 0,
            x_size: 0,
            y_size: 0,
            x_count_chars: 0,
            y_count_chars: 0,
            fill_attribute: 0,
            flags: 0,
            show_window: 0,
            reserved2_count: 0,
            reserved2: std::ptr::null_mut(),
            std_input: std::ptr::null_mut(),
            std_output: std::ptr::null_mut(),
            std_error: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ProcessInformation {
    process: *mut std::ffi::c_void,
    thread: *mut std::ffi::c_void,
    process_id: u32,
    thread_id: u32,
}

/// Windows commands are created suspended, assigned to the configured
/// kill-on-close job, then resumed. This closes the spawn-to-job-assignment
/// race: no child instruction runs outside containment.
#[cfg(windows)]
fn spawn_contained_command(
    spec: &CommandSpec,
) -> std::io::Result<(PlatformChild, ProcessContainment)> {
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    let job = WindowsJob::create()?;
    let (stdin_read, stdin_write) = create_pipe()?;
    // Closing the parent writer before creation gives every command a valid,
    // private EOF stdin even in GUI processes without an inheritable console.
    drop(stdin_write);
    let (stdout_read, stdout_write) = create_pipe()?;
    let (stderr_read, stderr_write) = create_pipe()?;
    set_handle_inheritable(stdout_read.as_raw_handle(), false)?;
    set_handle_inheritable(stderr_read.as_raw_handle(), false)?;
    let mut inherited_handles = [
        stdin_read.as_raw_handle(),
        stdout_write.as_raw_handle(),
        stderr_write.as_raw_handle(),
    ];
    let process_attributes = ProcessAttributeList::for_handle_list(&mut inherited_handles)?;
    let mut startup = StartupInfoExW {
        startup_info: StartupInfoW {
            flags: STARTF_USESTDHANDLES,
            std_input: stdin_read.as_raw_handle(),
            std_output: stdout_write.as_raw_handle(),
            std_error: stderr_write.as_raw_handle(),
            ..StartupInfoW::default()
        },
        attribute_list: process_attributes.as_ptr(),
    };
    // EXTENDED_STARTUPINFO_PRESENT requires cb to describe the complete
    // STARTUPINFOEXW structure, not only its STARTUPINFOW prefix.
    startup.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
    let mut process_info = ProcessInformation::default();
    let mut command_line = windows_command_line(spec)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            (&mut startup as *mut StartupInfoExW).cast::<StartupInfoW>(),
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: CreateProcessW returned these owned handles exactly once.
    let process = unsafe { OwnedHandle::from_raw_handle(process_info.process) };
    // SAFETY: CreateProcessW returned these owned handles exactly once.
    let thread = unsafe { OwnedHandle::from_raw_handle(process_info.thread) };
    if let Err(error) = job.assign(process.as_raw_handle()) {
        terminate_suspended_process(process.as_raw_handle());
        return Err(error);
    }
    if unsafe { ResumeThread(thread.as_raw_handle()) } == INVALID_DWORD {
        let error = std::io::Error::last_os_error();
        terminate_suspended_process(process.as_raw_handle());
        return Err(error);
    }
    drop(thread);
    drop(stdout_write);
    drop(stderr_write);
    Ok((
        PlatformChild {
            process,
            stdout: Some(File::from(stdout_read)),
            stderr: Some(File::from(stderr_read)),
        },
        ProcessContainment::from_job(job),
    ))
}

#[cfg(windows)]
fn create_pipe() -> std::io::Result<(
    std::os::windows::io::OwnedHandle,
    std::os::windows::io::OwnedHandle,
)> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    let attributes = SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor: std::ptr::null_mut(),
        inherit_handle: 1,
    };
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreatePipe returned one owned handle for each endpoint.
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    // SAFETY: CreatePipe returned one owned handle for each endpoint.
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    Ok((read, write))
}

#[cfg(windows)]
fn set_handle_inheritable(handle: *mut std::ffi::c_void, inheritable: bool) -> std::io::Result<()> {
    let flags = u32::from(inheritable) * HANDLE_FLAG_INHERIT;
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Owns the native attribute-list allocation for the entire CreateProcessW
/// call, deleting it on every success and failure path.
#[cfg(windows)]
struct ProcessAttributeList {
    attribute_list: *mut std::ffi::c_void,
    _storage: Vec<usize>,
}

#[cfg(windows)]
impl ProcessAttributeList {
    fn for_handle_list(handles: &mut [*mut std::ffi::c_void]) -> std::io::Result<Self> {
        let mut required_bytes = 0_usize;
        // The sizing call intentionally returns false and supplies the size.
        let _ = unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut required_bytes)
        };
        if required_bytes == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let word_size = std::mem::size_of::<usize>();
        let mut storage = vec![0_usize; required_bytes.div_ceil(word_size)];
        let attribute_list = storage.as_mut_ptr().cast::<std::ffi::c_void>();
        if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut required_bytes) }
            == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe {
            UpdateProcThreadAttribute(
                attribute_list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_mut_ptr().cast::<std::ffi::c_void>(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            // SAFETY: InitializeProcThreadAttributeList completed above.
            unsafe { DeleteProcThreadAttributeList(attribute_list) };
            return Err(error);
        }
        Ok(Self {
            attribute_list,
            _storage: storage,
        })
    }

    fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.attribute_list
    }
}

#[cfg(windows)]
impl Drop for ProcessAttributeList {
    fn drop(&mut self) {
        // SAFETY: this object is constructed only after successful
        // InitializeProcThreadAttributeList.
        unsafe { DeleteProcThreadAttributeList(self.attribute_list) };
    }
}

#[cfg(windows)]
fn terminate_suspended_process(process: *mut std::ffi::c_void) {
    let _ = unsafe { TerminateProcess(process, 1) };
    let _ = unsafe { WaitForSingleObject(process, INFINITE) };
}

#[cfg(windows)]
fn windows_command_line(spec: &CommandSpec) -> String {
    std::iter::once(&spec.program)
        .chain(spec.args.iter())
        .map(|argument| quote_windows_command_line_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn quote_windows_command_line_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument.contains(' ')
        && !argument.contains('\t')
        && !argument.contains('"')
    {
        return argument.into();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
struct PlatformChild {
    process: std::os::windows::io::OwnedHandle,
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
}

trait SupervisedChild {
    type Stdout: Read + Send + 'static;
    type Stderr: Read + Send + 'static;

    fn take_stdout(&mut self) -> Option<Self::Stdout>;
    fn take_stderr(&mut self) -> Option<Self::Stderr>;
    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
    fn kill(&mut self) -> std::io::Result<()>;
}

impl SupervisedChild for std::process::Child {
    type Stdout = std::process::ChildStdout;
    type Stderr = std::process::ChildStderr;

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        std::process::Child::wait(self)
    }

    fn kill(&mut self) -> std::io::Result<()> {
        std::process::Child::kill(self)
    }
}

#[cfg(windows)]
impl SupervisedChild for PlatformChild {
    type Stdout = std::fs::File;
    type Stderr = std::fs::File;

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn poll(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::ExitStatusExt;

        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut exit_code) } == 0
                {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(Some(std::process::ExitStatus::from_raw(exit_code)))
                }
            }
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        use std::os::windows::io::AsRawHandle;

        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(std::io::Error::last_os_error());
        }
        self.poll()?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "process wait completed without an exit status",
            )
        })
    }

    fn kill(&mut self) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;

        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

struct Supervision<'a, C: SupervisedChild> {
    child: C,
    containment: ProcessContainment,
    timeout: Duration,
    cancelled: &'a Mutex<bool>,
    cancellation: Option<&'a CancellationToken>,
    command_name: &'a str,
    cancellation_message: &'a str,
    capture_limit: usize,
    artifact_watch: Option<(&'a Path, ArtifactQuota)>,
}

fn supervise_command_child<C, F>(
    supervision: Supervision<'_, C>,
    mut poll: F,
) -> Result<CommandOutput>
where
    C: SupervisedChild,
    F: FnMut(&mut C) -> std::io::Result<Option<std::process::ExitStatus>>,
{
    let Supervision {
        mut child,
        containment,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        capture_limit,
        artifact_watch,
    } = supervision;
    let stdout = child.take_stdout().expect("stdout was configured as piped");
    let stderr = child.take_stderr().expect("stderr was configured as piped");
    let stdout_reader = drain_command_stream(stdout, capture_limit);
    let stderr_reader = drain_command_stream(stderr, capture_limit);
    let deadline = Instant::now() + timeout;
    loop {
        if *cancelled.lock().unwrap_or_else(|error| error.into_inner())
            || cancellation.is_some_and(CancellationToken::is_cancelled)
        {
            terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
            return Err(ScanError::Cancelled(cancellation_message.into()));
        }
        if let Some((directory, quota)) = artifact_watch {
            if let Err(error) = validate_artifact_quota(directory, quota) {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(error);
            }
        }
        match poll(&mut child) {
            Ok(Some(_)) => {
                let status = child.wait().map_err(|error| {
                    ScanError::Unsupported(format!("{command_name} output failed: {error}"))
                })?;
                if let Some((directory, quota)) = artifact_watch {
                    if let Err(error) = validate_artifact_quota(directory, quota) {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                }
                let stdout = match join_command_stream(stdout_reader) {
                    Ok(stdout) => stdout,
                    Err(error) => {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                };
                let stderr = match join_command_stream(stderr_reader) {
                    Ok(stderr) => stderr,
                    Err(error) => {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                };
                if stdout.overflowed || stderr.overflowed {
                    return Err(ScanError::Unsupported(format!(
                        "{command_name} output exceeded the {capture_limit} byte capture limit"
                    )));
                }
                return Ok(CommandOutput {
                    success: status.success(),
                    stdout: stdout.bytes,
                    stderr: stderr.bytes,
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(ScanError::Unsupported(format!("{command_name} timed out")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(ScanError::Unsupported(format!(
                    "{command_name} failed: {error}"
                )));
            }
        }
    }
}

fn terminate_child_and_readers(
    child: &mut impl SupervisedChild,
    containment: &ProcessContainment,
    stdout_reader: CommandStreamReader,
    stderr_reader: CommandStreamReader,
) {
    containment.terminate(child);
    let _ = child.wait();
    // Do not join here. A deliberately daemonized Unix descendant can retain
    // the inherited pipe after escaping its process group; dropping a handle
    // detaches the reader instead of making cancellation wait indefinitely.
    drop(stdout_reader);
    drop(stderr_reader);
}

struct DrainedCommandStream {
    bytes: Vec<u8>,
    overflowed: bool,
}

struct CommandStreamReader {
    result: Receiver<std::io::Result<DrainedCommandStream>>,
    thread: JoinHandle<()>,
}

fn drain_command_stream<R: Read + Send + 'static>(
    mut stream: R,
    capture_limit: usize,
) -> CommandStreamReader {
    let (sender, result) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let drained = (|| {
            let mut bytes = Vec::new();
            let mut overflowed = false;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    return Ok(DrainedCommandStream { bytes, overflowed });
                }
                let remaining = capture_limit.saturating_sub(bytes.len());
                let captured = remaining.min(count);
                bytes.extend_from_slice(&buffer[..captured]);
                overflowed |= captured < count;
            }
        })();
        let _ = sender.send(drained);
    });
    CommandStreamReader { result, thread }
}

fn join_command_stream(reader: CommandStreamReader) -> Result<DrainedCommandStream> {
    let result = reader
        .result
        .recv_timeout(COMMAND_STREAM_DRAIN_GRACE)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => ScanError::Unsupported(
                "command output streams remained open after the command exited".into(),
            ),
            mpsc::RecvTimeoutError::Disconnected => {
                ScanError::Unsupported("command output reader disconnected".into())
            }
        })?;
    reader
        .thread
        .join()
        .map_err(|_| ScanError::Unsupported("command output reader panicked".into()))?;
    result.map_err(|error| ScanError::Unsupported(format!("command output read failed: {error}")))
}

pub(crate) fn simulate_backends() -> bool {
    matches!(
        std::env::var("OPEN_SCANLINE_SIMULATE_BACKENDS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

pub(crate) fn parse_pipe_devices(
    text: &str,
    id_prefix: &str,
    display_prefix: &str,
    backend: &str,
) -> Vec<DeviceInfo> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (id, name) = line
                .split_once('|')
                .map(|(id, name)| (id.trim(), name.trim()))
                .unwrap_or((line, line));
            (!id.is_empty()).then(|| {
                DeviceInfo::new(
                    format!("{id_prefix}:{id}"),
                    format!("{display_prefix}: {name}"),
                    backend,
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::fs;

    #[test]
    fn document_batch_deadline_scales_and_remains_bounded() {
        assert_eq!(document_batch_timeout(1), Duration::from_secs(105));
        assert!(document_batch_timeout(100) > document_batch_timeout(2));
        assert_eq!(
            document_batch_timeout(u32::MAX),
            Duration::from_secs(43_200)
        );
    }

    #[cfg(windows)]
    #[test]
    fn suspended_windows_command_line_keeps_spaces_quotes_and_trailing_slashes_intact() {
        let command_line = windows_command_line(&CommandSpec {
            program: r"C:\Program Files\scanner.exe".into(),
            args: vec![
                "plain".into(),
                "with space".into(),
                "embedded\"quote".into(),
                r"C:\output folder\\".into(),
            ],
        });

        assert_eq!(
            command_line,
            r#""C:\Program Files\scanner.exe" plain "with space" "embedded\"quote" "C:\output folder\\""#
        );
    }

    #[cfg(windows)]
    #[test]
    fn startup_info_ex_has_the_windows_abi_prefix_and_attribute_pointer() {
        assert_eq!(std::mem::offset_of!(StartupInfoExW, startup_info), 0);
        assert_eq!(
            std::mem::offset_of!(StartupInfoExW, attribute_list),
            std::mem::size_of::<StartupInfoW>()
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<StartupInfoW>(), 104);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(std::mem::size_of::<StartupInfoW>(), 68);
    }

    #[test]
    fn external_token_cancels_a_running_child_well_before_its_timeout() {
        let token = CancellationToken::new();
        let trigger = token.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            trigger.cancel();
        });
        let local = Mutex::new(false);
        let started = Instant::now();
        let result = run_command_with_cancellation(
            &CommandSpec {
                program: "sleep".into(),
                args: vec!["5".into()],
            },
            Duration::from_secs(5),
            &local,
            Some(&token),
            "fixture command",
            "fixture cancelled",
        );
        canceller.join().unwrap();

        assert!(
            matches!(result, Err(ScanError::Cancelled(message)) if message == "fixture cancelled")
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancellation should not wait for the child timeout"
        );
    }

    #[cfg(unix)]
    #[test]
    fn try_wait_error_kills_reaps_and_detaches_stream_readers() {
        let (child, containment) = spawn_contained_command(&CommandSpec {
            program: "sleep".into(),
            args: vec!["5".into()],
        })
        .unwrap();
        let pid = child.id() as i32;
        let local = Mutex::new(false);
        let error = supervise_command_child(
            Supervision {
                child,
                containment,
                timeout: Duration::from_secs(5),
                cancelled: &local,
                cancellation: None,
                command_name: "fixture command",
                cancellation_message: "fixture cancelled",
                capture_limit: COMMAND_STREAM_CAPTURE_LIMIT,
                artifact_watch: None,
            },
            |_| Err(std::io::Error::other("forced poll failure")),
        )
        .unwrap_err();

        assert!(error.to_string().contains("forced poll failure"));
        let status = unsafe { libc::kill(pid, 0) };
        assert_eq!(
            status, -1,
            "child process remained alive after poll failure"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    fn fixture_with_pipe_holding_descendant() -> (
        CommandSpec,
        TemporaryOutput,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let output = TemporaryOutput::new("process-tree-fixture", "pid").unwrap();
        let parent_pid = output.path().to_path_buf();
        let descendant_pid = output.directory().join("descendant.pid");
        let script = concat!(
            "printf '%s\\n' \"$$\" > \"$1\"; ",
            "sh -c 'printf \"%s\\n\" \"$$\" > \"$1\"; sleep 30' sh \"$2\" & ",
            "sleep 30"
        );
        (
            CommandSpec {
                program: "sh".into(),
                args: vec![
                    "-c".into(),
                    script.into(),
                    "process-tree-fixture".into(),
                    parent_pid.display().to_string(),
                    descendant_pid.display().to_string(),
                ],
            },
            output,
            parent_pid,
            descendant_pid,
        )
    }

    #[cfg(unix)]
    fn wait_for_fixture_pid(path: &Path) -> libc::pid_t {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(pid) = fs::read_to_string(path).and_then(|pid| {
                pid.trim()
                    .parse::<libc::pid_t>()
                    .map_err(std::io::Error::other)
            }) {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not write its PID to {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn assert_pid_gone(pid: libc::pid_t) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "fixture process {pid} remained alive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_a_descendant_that_keeps_both_output_pipes_open() {
        let (spec, _output, parent_pid_path, descendant_pid_path) =
            fixture_with_pipe_holding_descendant();
        let token = CancellationToken::new();
        let trigger = token.clone();
        let cancellable_descendant_pid_path = descendant_pid_path.clone();
        let canceller = std::thread::spawn(move || {
            let _ = wait_for_fixture_pid(&cancellable_descendant_pid_path);
            trigger.cancel();
        });
        let local = Mutex::new(false);
        let started = Instant::now();
        let error = run_command_with_cancellation(
            &spec,
            Duration::from_secs(10),
            &local,
            Some(&token),
            "fixture command",
            "fixture cancelled",
        )
        .unwrap_err();
        canceller.join().unwrap();

        assert!(matches!(error, ScanError::Cancelled(message) if message == "fixture cancelled"));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_pid_gone(wait_for_fixture_pid(&parent_pid_path));
        assert_pid_gone(wait_for_fixture_pid(&descendant_pid_path));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_a_descendant_that_keeps_both_output_pipes_open() {
        let (spec, _output, parent_pid_path, descendant_pid_path) =
            fixture_with_pipe_holding_descendant();
        let local = Mutex::new(false);
        let started = Instant::now();
        let error = run_command_with_cancellation(
            &spec,
            // Leave enough startup headroom for loaded CI hosts while keeping
            // the timeout far below the fixture's 30-second lifetime.
            Duration::from_secs(1),
            &local,
            None,
            "fixture command",
            "fixture cancelled",
        )
        .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_pid_gone(wait_for_fixture_pid(&parent_pid_path));
        assert_pid_gone(wait_for_fixture_pid(&descendant_pid_path));
    }

    #[test]
    fn artifact_quota_is_checked_from_request_pages_and_image_limits() {
        let request = ScanRequest {
            width: 2,
            height: 3,
            pixel_format: crate::core::PixelFormat::Rgb8,
            ..Default::default()
        };
        let quota = artifact_quota_for_request(&request, 4).unwrap();

        assert_eq!(quota.max_files, 4);
        assert_eq!(quota.max_bytes, (18 + ARTIFACT_PER_IMAGE_OVERHEAD) * 4);
        assert!(artifact_quota_for_request(
            &ScanRequest {
                width: crate::core::MAX_IMAGE_DIMENSION + 1,
                ..request
            },
            1,
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn artifact_quota_rejects_nested_directories_and_symlinked_outputs() {
        use std::os::unix::fs::symlink;

        let nested = TemporaryOutput::new("quota-nested-fixture", "png").unwrap();
        fs::create_dir(nested.directory().join("unexpected")).unwrap();
        assert!(matches!(
            validate_artifact_quota(
                nested.directory(),
                ArtifactQuota {
                    max_files: 1,
                    max_bytes: 1024
                }
            ),
            Err(ScanError::Unsupported(message)) if message.contains("nested directory")
        ));

        let symlinked = TemporaryOutput::new("quota-symlink-fixture", "png").unwrap();
        let external = std::env::temp_dir().join(format!(
            ".open-scanline-quota-symlink-source-{}-{}",
            std::process::id(),
            ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&external, vec![0_u8; 4096]).unwrap();
        symlink(&external, symlinked.path()).unwrap();
        assert!(matches!(
            validate_artifact_quota(
                symlinked.directory(),
                ArtifactQuota {
                    max_files: 1,
                    max_bytes: 1024
                }
            ),
            Err(ScanError::Unsupported(message)) if message.contains("non-regular entry")
        ));
        fs::remove_file(external).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn artifact_quota_breach_kills_reaps_tree_and_temporary_output_is_removed() {
        let output = TemporaryOutput::new("quota-process-tree-fixture", "png").unwrap();
        let directory = output.directory().to_path_buf();
        let parent_pid_path = directory.join("parent.pid");
        let descendant_pid_path = directory.join("descendant.pid");
        let spec = CommandSpec {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                concat!(
                    "printf '%s\\n' \"$$\" > \"$1\"; ",
                    "sh -c 'printf \"%s\\n\" \"$$\" > \"$1\"; sleep 30' sh \"$2\" & ",
                    "while [ ! -s \"$2\" ]; do :; done; ",
                    "dd if=/dev/zero of=\"$3\" bs=128 count=1 2>/dev/null; sleep 30"
                )
                .into(),
                "quota-process-tree-fixture".into(),
                parent_pid_path.display().to_string(),
                descendant_pid_path.display().to_string(),
                output.path().display().to_string(),
            ],
        };
        let local = Mutex::new(false);
        let error = run_contained_command_with_artifact_quota(
            &spec,
            Duration::from_secs(5),
            &local,
            None,
            "fixture command",
            "fixture cancelled",
            ArtifactWatch {
                directory: output.directory(),
                quota: ArtifactQuota {
                    max_files: 8,
                    max_bytes: 64,
                },
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("artifact output exceeded"));
        assert_pid_gone(wait_for_fixture_pid(&parent_pid_path));
        assert_pid_gone(wait_for_fixture_pid(&descendant_pid_path));
        drop(output);
        assert!(
            !directory.exists(),
            "temporary artifact directory survived quota rejection"
        );
    }
}
