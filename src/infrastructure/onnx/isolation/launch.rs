use super::paths;
use super::{OnnxInferenceOptions, ONNX_WORKER_PROTOCOL};
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::TemporaryOutput;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const ONNX_WALL_TIMEOUT: Duration = Duration::from_secs(30);
const ONNX_WORKER_MEMORY_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SAMPLER_FAILURE_REAP_GRACE: Duration = Duration::from_millis(50);
const SAMPLER_FAILURE_REAP_INTERVAL: Duration = Duration::from_millis(5);

pub(super) fn run_isolated_onnx_with_worker(
    executable: &Path,
    working_directory: &Path,
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
) -> Result<super::super::OnnxReport> {
    let report = TemporaryOutput::new("onnx-report", "json")?;
    let status = supervise_worker(
        executable,
        working_directory,
        &worker_args(input, model, report.path(), options),
        ONNX_WALL_TIMEOUT,
    )?;
    if !status.success() {
        return Err(ScanError::Other(format!(
            "ONNX worker exited with status {status}"
        )));
    }
    let metadata = std::fs::metadata(report.path())
        .map_err(|error| ScanError::Other(format!("ONNX worker produced no report: {error}")))?;
    paths::check_report_size(metadata.len())?;
    let bytes = std::fs::read(report.path())?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ScanError::Other(format!("invalid ONNX worker report: {error}")))
}

fn worker_args(
    input: &Path,
    model: &Path,
    report: &Path,
    options: &OnnxInferenceOptions,
) -> Vec<String> {
    let mut args = vec![
        "__onnx-worker".into(),
        "--in".into(),
        input.to_string_lossy().into_owned(),
        "--model".into(),
        model.to_string_lossy().into_owned(),
        "--report".into(),
        report.to_string_lossy().into_owned(),
        "--layout".into(),
        layout_arg(options.layout).into(),
        "--normalization".into(),
        normalization_arg(options.normalization).into(),
        "--worker-protocol".into(),
        ONNX_WORKER_PROTOCOL.into(),
    ];
    if let Some(input_name) = options.input_name.as_deref() {
        args.push("--input-name".into());
        args.push(input_name.into());
    }
    args
}

fn layout_arg(layout: super::super::OnnxInputLayout) -> &'static str {
    match layout {
        super::super::OnnxInputLayout::Auto => "auto",
        super::super::OnnxInputLayout::Nchw => "nchw",
        super::super::OnnxInputLayout::Nhwc => "nhwc",
    }
}
fn normalization_arg(normalization: super::super::OnnxNormalization) -> &'static str {
    match normalization {
        super::super::OnnxNormalization::ZeroToOne => "zero-to-one",
        super::super::OnnxNormalization::None => "none",
    }
}

pub(super) fn start_parent_liveness_watchdog() {
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
    working_directory: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::ExitStatus> {
    #[cfg(target_os = "macos")]
    let sampler = |pid| worker_resident_bytes(pid).map(Some);
    #[cfg(not(target_os = "macos"))]
    let sampler = |_| Ok(None);
    supervise_worker_with_memory_limit_and_sampler(
        executable,
        working_directory,
        args,
        timeout,
        ONNX_WORKER_MEMORY_LIMIT_BYTES,
        sampler,
    )
}

fn supervise_worker_with_memory_limit_and_sampler<F>(
    executable: &Path,
    working_directory: &Path,
    args: &[String],
    timeout: Duration,
    memory_limit: u64,
    sampler: F,
) -> Result<std::process::ExitStatus>
where
    F: FnMut(u32) -> std::io::Result<Option<u64>>,
{
    let mut child = spawn_worker(executable, working_directory, args)?;
    let _liveness = child.stdin.take();
    supervise_child(&mut child, timeout, memory_limit, sampler)
}

fn spawn_worker(
    executable: &Path,
    working_directory: &Path,
    args: &[String],
) -> Result<std::process::Child> {
    Command::new(executable)
        .args(args)
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ScanError::Other(format!("ONNX worker failed to start: {error}")))
}

fn supervise_child<F>(
    child: &mut std::process::Child,
    timeout: Duration,
    memory_limit: u64,
    mut sampler: F,
) -> Result<std::process::ExitStatus>
where
    F: FnMut(u32) -> std::io::Result<Option<u64>>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() >= deadline => {
                return terminate_with(
                    child,
                    ScanError::Other(format!(
                        "ONNX worker exceeded the {}-second wall-time limit",
                        timeout.as_secs()
                    )),
                )
            }
            Ok(None) => match sample_worker(child, memory_limit, &mut sampler) {
                Ok(()) => std::thread::sleep(Duration::from_millis(25)),
                Err(error) => return terminate_with(child, error),
            },
            Err(error) => {
                return terminate_with(
                    child,
                    ScanError::Other(format!("ONNX worker supervision failed: {error}")),
                )
            }
        }
    }
}

fn sample_worker<F>(
    child: &mut std::process::Child,
    memory_limit: u64,
    sampler: &mut F,
) -> Result<()>
where
    F: FnMut(u32) -> std::io::Result<Option<u64>>,
{
    match sampler(child.id()) {
        Ok(Some(resident)) if resident > memory_limit => Err(ScanError::Unsupported(format!(
            "ONNX worker exceeded the {} MiB resident-memory limit",
            memory_limit / (1024 * 1024)
        ))),
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(status) = reap_natural_exit_after_sampler_failure(child) {
                let _ = status;
                Ok(())
            } else {
                Err(ScanError::Unsupported(format!("ONNX worker containment is unavailable: could not sample resident memory: {error}")))
            }
        }
    }
}

fn terminate_with(
    child: &mut std::process::Child,
    error: ScanError,
) -> Result<std::process::ExitStatus> {
    let _ = child.kill();
    let _ = child.wait();
    Err(error)
}
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
