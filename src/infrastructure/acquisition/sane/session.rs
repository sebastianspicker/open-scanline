use super::capabilities::{
    parse_scanimage_help, parse_scanimage_maintenance_options, scanimage_maintenance_command,
    scanimage_maintenance_inspection_command, SaneCapabilities, SaneMaintenanceAction,
    SaneMaintenanceOptions,
};
use super::command::{build_scanimage_command, selected_source};
use super::probing::{which, SystemCommandRunner};
use super::{emit_simulated_pages, emit_single_pages};
use crate::domain::acquisition::reject_single_page_duplex;
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::batch::{batch_timeout, probe_timeout};
use crate::infrastructure::acquisition::command_backend::{
    mentions_feeder_exhausted, numbered_page_outputs,
};
use crate::infrastructure::acquisition::{
    opens_simulated_session, simulate_backends, DeviceInfo, DeviceMaintenanceCapabilities,
    DeviceSession, ScanPagesResult,
};
use crate::infrastructure::media::NativeImageDecoder;
use crate::infrastructure::runtime::{
    artifact_quota_for_request, validate_artifact_quota, ArtifactQuota, ArtifactWatch,
    CommandOutput, CommandRunner, CommandSession, CommandSpec, ImageDecoder, TemporaryOutput,
};
use crate::operation::CancellationToken;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Sets the flag on drop, so a panic still stops the scanimage child.
struct StopOnDrop<'a>(&'a Mutex<bool>);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = true;
    }
}

pub struct SaneDeviceSession {
    pub device_id: String,
    /// SANE device name for -d
    sane_name: String,
    simulate: bool,
    session: CommandSession,
    runner: Arc<dyn CommandRunner>,
    decoder: Arc<dyn ImageDecoder>,
    inspect_capabilities: bool,
    capabilities: OnceLock<Option<SaneCapabilities>>,
    /// Help inventories read with `--source=<name>`, see `capabilities_for_source`.
    source_capabilities: Box<Mutex<Vec<(String, SaneCapabilities)>>>,
    maintenance_options: OnceLock<Option<SaneMaintenanceOptions>>,
}

impl SaneDeviceSession {
    pub fn new(device_id: String, sane_name: String, simulate: bool) -> Self {
        Self {
            device_id,
            sane_name,
            simulate,
            session: CommandSession::default(),
            runner: Arc::new(SystemCommandRunner),
            decoder: Arc::new(NativeImageDecoder),
            inspect_capabilities: !simulate,
            capabilities: OnceLock::new(),
            source_capabilities: Box::default(),
            maintenance_options: OnceLock::new(),
        }
    }

    pub fn new_with_adapters(
        device_id: String,
        sane_name: String,
        simulate: bool,
        runner: Arc<dyn CommandRunner>,
        decoder: Arc<dyn ImageDecoder>,
    ) -> Self {
        Self {
            device_id,
            sane_name,
            simulate,
            session: CommandSession::default(),
            runner,
            decoder,
            inspect_capabilities: false,
            capabilities: OnceLock::new(),
            source_capabilities: Box::default(),
            maintenance_options: OnceLock::new(),
        }
    }

    /// Capabilities for a real acquisition. Without readable device options no
    /// source, colour mode, or duplex value can be chosen safely, so a session
    /// that inspects its device refuses to guess them.
    fn acquisition_capabilities(
        &self,
        binary: &Path,
        request: &ScanRequest,
    ) -> Result<Option<SaneCapabilities>> {
        let Some(capabilities) = self.capabilities(binary) else {
            if self.inspect_capabilities {
                return Err(ScanError::Unsupported(format!(
                    "could not read the options of SANE device {name}: `scanimage --help -d {name}` \
                     failed or listed none; run it to check the device, or rerun with --verbose",
                    name = self.sane_name
                )));
            }
            return Ok(None);
        };
        Ok(Some(self.capabilities_for_source(
            binary,
            capabilities,
            request,
        )))
    }

    /// Page size and duplex options are `[inactive]` in the help of a source
    /// that lacks them. When the request needs another source than the one the
    /// help was read with, read the help again with that source selected.
    fn capabilities_for_source(
        &self,
        binary: &Path,
        base: &SaneCapabilities,
        request: &ScanRequest,
    ) -> SaneCapabilities {
        let Some(source) = base
            .source_dependent_options
            .then(|| selected_source(base, request).ok().flatten())
            .flatten()
            .filter(|source| {
                !base
                    .default_source
                    .as_deref()
                    .is_some_and(|default| default.eq_ignore_ascii_case(source))
            })
        else {
            return base.clone();
        };
        let Ok(mut cache) = self.source_capabilities.lock() else {
            return base.clone();
        };
        if let Some((_, capabilities)) = cache.iter().find(|(name, _)| *name == source) {
            return capabilities.clone();
        }
        // A failed read is cached too, so the probe is not repeated per scan.
        let capabilities = self
            .read_help(binary, Some(&source))
            .unwrap_or_else(|| base.clone());
        cache.push((source, capabilities.clone()));
        capabilities
    }

    fn capabilities(&self, binary: &Path) -> Option<&SaneCapabilities> {
        self.capabilities
            .get_or_init(|| {
                if !self.inspect_capabilities {
                    return None;
                }
                self.read_help(binary, None)
            })
            .as_ref()
    }

    /// `scanimage --help -d <device> [--source=<source>]`, parsed.
    fn read_help(&self, binary: &Path, source: Option<&str>) -> Option<SaneCapabilities> {
        let mut args = vec!["--help".into(), "-d".into(), self.sane_name.clone()];
        args.extend(source.map(|source| format!("--source={source}")));
        let spec = CommandSpec {
            program: binary.display().to_string(),
            args,
        };
        let output = self
            .runner
            .run(
                &spec,
                probe_timeout(Duration::from_secs(8)),
                self.session.cancelled(),
                self.session.cancellation_token().as_ref(),
                None,
            )
            .inspect_err(|error| {
                crate::debug_log::debug_log!("sane", "scanimage --help failed: {error}");
            })
            .ok()?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        let capabilities = parse_scanimage_help(&text);
        if capabilities.is_empty() {
            crate::debug_log::debug_log!("sane", "capabilities empty");
        }
        (!capabilities.is_empty()).then_some(capabilities)
    }

    fn maintenance_options(&self, binary: &Path) -> Option<&SaneMaintenanceOptions> {
        self.maintenance_options
            .get_or_init(|| {
                let spec = scanimage_maintenance_inspection_command(binary, &self.sane_name);
                let output = self
                    .runner
                    .run(
                        &spec,
                        probe_timeout(Duration::from_secs(8)),
                        self.session.cancelled(),
                        self.session.cancellation_token().as_ref(),
                        None,
                    )
                    .ok()?;
                if !output.success {
                    return None;
                }
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                Some(parse_scanimage_maintenance_options(&text))
            })
            .as_ref()
    }

    fn maintenance_capabilities_result(&self) -> DeviceMaintenanceCapabilities {
        if self.simulate {
            return DeviceMaintenanceCapabilities::unsupported(
                "SANE simulation does not emulate scanner maintenance",
            );
        }
        if self.session.is_closed() {
            return DeviceMaintenanceCapabilities::unsupported("SANE session is closed");
        }
        let Some(binary) = which("scanimage") else {
            return DeviceMaintenanceCapabilities::unsupported(
                "SANE maintenance requires scanimage CLI",
            );
        };
        self.maintenance_options(&binary)
            .map(SaneMaintenanceOptions::capabilities)
            .unwrap_or_else(|| {
                DeviceMaintenanceCapabilities::unsupported(
                    "could not inspect SANE maintenance options",
                )
            })
    }

    fn run_maintenance(&self, action: SaneMaintenanceAction) -> serde_json::Value {
        use serde_json::json;
        if self.session.is_closed() {
            return crate::infrastructure::acquisition::contract::closed_session_envelope("sane");
        }
        if self.session.is_cancelled() {
            return json!({"ok": false, "status": "cancelled", "backend": "sane"});
        }
        let spec = match self.maintenance_command(action) {
            Ok(spec) => spec,
            Err(error) => return unsupported_maintenance(error),
        };
        maintenance_command_result(
            action,
            self.runner.run(
                &spec,
                Duration::from_secs(20),
                self.session.cancelled(),
                self.session.cancellation_token().as_ref(),
                None,
            ),
        )
    }

    fn maintenance_command(&self, action: SaneMaintenanceAction) -> Result<CommandSpec> {
        let binary = which("scanimage").ok_or_else(|| {
            ScanError::Unsupported("SANE maintenance requires scanimage CLI".into())
        })?;
        let options = self.maintenance_options(&binary).ok_or_else(|| {
            ScanError::Unsupported("could not inspect SANE maintenance options".into())
        })?;
        scanimage_maintenance_command(&binary, &self.sane_name, options, action)
    }

    fn acquire_scanimage(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        let Some(bin) = which("scanimage") else {
            return Err(ScanError::Unsupported(
                "SANE scan requires scanimage CLI".into(),
            ));
        };
        let output_file = TemporaryOutput::new("sane", "png")?;
        let artifact_quota = artifact_quota_for_request(request, 1)?;
        let spec = build_scanimage_command(
            &bin,
            &self.sane_name,
            request,
            output_file.path(),
            self.acquisition_capabilities(&bin, request)?.as_ref(),
        )?;
        super::super::batch::run_materialized_scan_command(
            &self.session,
            self.runner.as_ref(),
            self.decoder.as_ref(),
            &spec,
            &output_file,
            artifact_quota,
            "scanimage failed",
        )
    }

    fn acquire_scanimage_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        let binary = which("scanimage")
            .ok_or_else(|| ScanError::Unsupported("SANE scan requires scanimage CLI".into()))?;
        let output = TemporaryOutput::new("sane-batch", "png")?;
        let quota = artifact_quota_for_request(request, max_pages)?;
        let spec = self.batch_scanimage_command(&binary, request, &output, max_pages)?;
        self.stream_scanimage_pages(&spec, &output, quota, max_pages, emit)
    }

    /// Run a `scanimage` batch on a scoped thread and emit each page as soon
    /// as scanimage renames its `.part` file, deleting it once emitted, so the
    /// artifact folder only holds the pages not yet consumed. When the command
    /// fails (jam, open cover, timeout, quota) the finished pages are still
    /// emitted, then the command's error is returned.
    fn stream_scanimage_pages(
        &self,
        spec: &CommandSpec,
        output: &TemporaryOutput,
        quota: ArtifactQuota,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        // The child watches this flag rather than the session's, so a stop the
        // stream triggers itself never leaves the session cancelled.
        let stop = Mutex::new(false);
        let token = self.session.cancellation_token();
        let (runner, directory) = (self.runner.as_ref(), output.directory());
        let mut emitted = 0_u32;
        let mut failure = None;
        let command = std::thread::scope(|scope| {
            let child = scope.spawn(|| {
                runner.run(
                    spec,
                    batch_timeout(max_pages),
                    &stop,
                    token.as_ref(),
                    Some(ArtifactWatch { directory, quota }),
                )
            });
            // A panic in `emit` or the decoder must still stop the child.
            let _stop_on_drop = StopOnDrop(&stop);
            while !child.is_finished() && !self.session.is_cancelled() {
                if let Err(error) =
                    self.emit_finished_pages(directory, max_pages, &mut emitted, emit)
                {
                    failure = Some(error);
                    break;
                }
                std::thread::sleep(BATCH_POLL_INTERVAL);
            }
            *stop.lock().unwrap_or_else(|error| error.into_inner()) = true;
            child
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
        if let Some(error) = failure {
            return Err(error);
        }
        if self.session.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let command = command.and_then(|command| {
            validate_artifact_quota(directory, quota)?;
            validate_batch_command(&command)
        });
        let swept = self.emit_finished_pages(directory, max_pages, &mut emitted, emit);
        command?;
        swept?;
        Ok(super::super::command_backend::command_backed_pages_result(
            emitted, max_pages,
        ))
    }

    fn batch_scanimage_command(
        &self,
        binary: &Path,
        request: &ScanRequest,
        output: &TemporaryOutput,
        max_pages: u32,
    ) -> Result<CommandSpec> {
        let mut spec = build_scanimage_command(
            binary,
            &self.sane_name,
            request,
            output.path(),
            self.acquisition_capabilities(binary, request)?.as_ref(),
        )?;
        spec.args
            .retain(|argument| !argument.starts_with("--output-file="));
        spec.args.extend([
            format!(
                "--batch={}",
                output.directory().join("page_%06d.png").display()
            ),
            "--batch-start=1".into(),
            format!("--batch-count={max_pages}"),
        ]);
        Ok(spec)
    }

    /// Emit, then delete, every complete page now in `directory`, in page
    /// order. More pages than requested over the whole run is an error.
    fn emit_finished_pages(
        &self,
        directory: &Path,
        max_pages: u32,
        emitted: &mut u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<()> {
        let pages = numbered_page_outputs(directory, "png")?;
        super::super::command_backend::validate_batch_page_count(
            *emitted as usize + pages.len(),
            max_pages,
            "scanimage",
        )?;
        let success = CommandOutput {
            success: true,
            exit_code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        for page in pages {
            let image = self.session.decode_materialized_output(
                &success,
                &page,
                self.decoder.as_ref(),
                "scanimage batch failed",
            )?;
            emit(image)?;
            *emitted += 1;
            std::fs::remove_file(&page)?;
        }
        Ok(())
    }
}

fn unsupported_maintenance(error: ScanError) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "status": "unsupported",
        "backend": "sane",
        "error": error.to_string(),
    })
}

fn maintenance_command_result(
    action: SaneMaintenanceAction,
    result: Result<CommandOutput>,
) -> serde_json::Value {
    match result {
        Ok(output) if output.success => successful_maintenance(action),
        Ok(output) => serde_json::json!({
            "ok": false,
            "status": "error",
            "backend": "sane",
            "error": String::from_utf8_lossy(&output.stderr).trim(),
        }),
        Err(ScanError::Cancelled(error)) => {
            serde_json::json!({"ok": false, "status": "cancelled", "backend": "sane", "error": error})
        }
        Err(error) => serde_json::json!({
            "ok": false,
            "status": "error",
            "backend": "sane",
            "error": error.to_string(),
        }),
    }
}

fn successful_maintenance(action: SaneMaintenanceAction) -> serde_json::Value {
    match action {
        SaneMaintenanceAction::Calibrate => {
            serde_json::json!({"ok": true, "status": "calibrated", "backend": "sane"})
        }
        SaneMaintenanceAction::FocusCenter => {
            serde_json::json!({"ok": true, "status": "focused", "backend": "sane", "mode": "center"})
        }
        SaneMaintenanceAction::FocusPoint {
            x_fraction,
            y_fraction,
        } => serde_json::json!({
            "ok": true,
            "status": "focused",
            "backend": "sane",
            "mode": "point",
            "x_frac": x_fraction,
            "y_frac": y_fraction,
        }),
    }
}

/// `scanimage` exits with the SANE status of the failed call; with
/// `--batch-count` set, an emptied feeder ends the job with this status.
const SANE_STATUS_NO_DOCS: i32 = 7;

/// How often a running batch is checked for finished pages and cancellation.
const BATCH_POLL_INTERVAL: Duration = Duration::from_millis(50);

fn validate_batch_command(command: &CommandOutput) -> Result<()> {
    if command.success
        || command.exit_code == Some(SANE_STATUS_NO_DOCS)
        || mentions_feeder_exhausted(&String::from_utf8_lossy(&command.stderr))
    {
        return Ok(());
    }
    Err(ScanError::Unsupported(format!(
        "scanimage batch failed: {}",
        String::from_utf8_lossy(&command.stderr).trim()
    )))
}

impl DeviceSession for SaneDeviceSession {
    fn maintenance_capabilities(&self) -> DeviceMaintenanceCapabilities {
        self.maintenance_capabilities_result()
    }

    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        super::super::batch::validate_session_state(
            self.session.is_closed(),
            self.session.is_cancelled(),
        )?;
        if self.simulate {
            return super::super::mock::MockDeviceSession::gradient(request);
        }
        self.acquire_scanimage(request)
    }

    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        super::super::batch::validate_batch_request(
            request,
            max_pages,
            self.session.is_closed(),
            self.session.is_cancelled(),
        )?;
        if self.simulate {
            return emit_simulated_pages(request, max_pages, emit);
        }
        self.scan_hardware_pages(request, max_pages, emit)
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn bind_cancellation(&self, token: CancellationToken) {
        self.session.bind_cancellation(token);
    }

    fn close(&self) {
        self.session.close();
    }

    fn calibrate(&self) -> serde_json::Value {
        self.run_maintenance(SaneMaintenanceAction::Calibrate)
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        let capabilities = self.maintenance_capabilities_result();
        let action = if capabilities.focus.supports_point() {
            SaneMaintenanceAction::FocusPoint {
                x_fraction: x,
                y_fraction: y,
            }
        } else if capabilities.focus.is_available() && x == 0.5 && y == 0.5 {
            SaneMaintenanceAction::FocusCenter
        } else {
            return serde_json::json!({
                "ok": false,
                "status": "unsupported",
                "backend": "sane",
                "x_frac": x,
                "y_frac": y,
                "error": "SANE backend only advertises centre focus; point focus was not run",
            });
        };
        self.run_maintenance(action)
    }
}

impl SaneDeviceSession {
    fn scan_hardware_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        if request.mode == ScanMode::Document {
            return self.acquire_scanimage_pages(request, max_pages, emit);
        }
        // A sheet-fed device scans a flatbed-style batch as one feeder job, so
        // an emptied feeder ends it normally instead of failing a single scan.
        if let Some(binary) = which("scanimage") {
            let feeder_only = self
                .capabilities(&binary)
                .is_some_and(super::command::feeder_only);
            if request.mode == ScanMode::Reflective && !request.duplex && feeder_only {
                let feeder_request = ScanRequest {
                    mode: ScanMode::Document,
                    ..request.clone()
                };
                return self.acquire_scanimage_pages(&feeder_request, max_pages, emit);
            }
        }
        if request.duplex {
            return Err(ScanError::Unsupported(
                "SANE duplex acquisition requires an ADF source".into(),
            ));
        }
        emit_single_pages(request, max_pages, emit, |page| {
            self.acquire_scanimage(page)
        })
    }
}

/// Open SANE session for listed device or sim.
pub fn open(device_id: &str) -> Result<SaneDeviceSession> {
    open_with_cancellation(device_id, None)
}

pub fn open_with_cancellation(
    device_id: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<SaneDeviceSession> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    let id = device_id.trim();
    if !id.starts_with("sane:") && id != "sane" {
        return Err(ScanError::DeviceNotFound(format!(
            "not a SANE device id: {id}"
        )));
    }
    if opens_simulated_session(id, "sane:sim", simulate_backends()) {
        return Ok(SaneDeviceSession::new(id.into(), "sim".into(), true));
    }
    let listed = list_for_open(
        id,
        || super::probing::recent_devices(cancellation),
        || super::probing::list_devices_with_cancellation(cancellation),
    );
    if !listed.iter().any(|d| d.id == id) {
        return Err(unknown_device(id, &listed));
    }
    let name = id.strip_prefix("sane:").unwrap_or(id).to_string();
    Ok(SaneDeviceSession::new(id.into(), name, false))
}

/// Devices to check `id` against. A miss in a reused listing may predate a
/// replug (USB names change with it), so only then list again.
fn list_for_open(
    id: &str,
    recent: impl FnOnce() -> (Vec<DeviceInfo>, bool),
    list: impl FnOnce() -> Vec<DeviceInfo>,
) -> Vec<DeviceInfo> {
    let (listed, reused) = recent();
    if reused && !listed.iter().any(|device| device.id == id) {
        return list();
    }
    listed
}

fn unknown_device(id: &str, listed: &[DeviceInfo]) -> ScanError {
    let ids = listed
        .iter()
        .map(|device| device.id.as_str())
        .collect::<Vec<_>>();
    let available = if ids.is_empty() {
        "SANE lists no devices; check `scanimage -L`".to_string()
    } else {
        format!("SANE lists: {}", ids.join(", "))
    };
    ScanError::DeviceNotFound(format!("unknown SANE device: {id}; {available}"))
}

#[cfg(test)]
mod tests {
    use super::{validate_batch_command, CommandOutput, SaneDeviceSession};
    use crate::domain::image::{ImageBuffer, PixelFormat};
    use crate::error::{Result, ScanError};
    use crate::infrastructure::acquisition::{DeviceSession, ScanPagesResult};
    use crate::infrastructure::runtime::{
        ArtifactQuota, ArtifactWatch, CommandRunner, CommandSpec, ImageDecoder, TemporaryOutput,
    };
    use crate::operation::CancellationToken;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Writes `pages` batch pages the way scanimage does (`.part`, then a
    /// rename). Then returns `result`, or with `None` waits for the stop flag.
    struct PageRunner {
        pages: u32,
        result: Option<CommandOutput>,
        stopped: AtomicBool,
    }

    impl PageRunner {
        fn new(pages: u32, result: Option<CommandOutput>) -> Arc<Self> {
            Arc::new(Self {
                pages,
                result,
                stopped: AtomicBool::new(false),
            })
        }
    }

    impl CommandRunner for PageRunner {
        fn run(
            &self,
            _: &CommandSpec,
            _: Duration,
            cancelled: &Mutex<bool>,
            _: Option<&CancellationToken>,
            artifacts: Option<ArtifactWatch<'_>>,
        ) -> Result<CommandOutput> {
            let directory = artifacts
                .expect("a batch runs with an artifact quota")
                .directory;
            let is_stopped = || *cancelled.lock().unwrap();
            for number in 1..=self.pages {
                if is_stopped() {
                    break;
                }
                let page = directory.join(format!("page_{number:06}.png"));
                let part = page.with_extension("png.part");
                std::fs::write(&part, b"x").unwrap();
                std::thread::sleep(Duration::from_millis(20));
                std::fs::rename(&part, &page).unwrap();
            }
            if let Some(result) = &self.result {
                return Ok(result.clone());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while !is_stopped() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.stopped.store(is_stopped(), Ordering::SeqCst);
            Err(ScanError::Cancelled("stopped".into()))
        }
    }

    /// Records the file name of every decoded page.
    #[derive(Default)]
    struct RecordingDecoder(Mutex<Vec<String>>);

    impl ImageDecoder for RecordingDecoder {
        fn decode(&self, path: &Path) -> Result<ImageBuffer> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            self.0.lock().unwrap().push(name);
            ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![0, 0, 0])
        }
    }

    const QUOTA: ArtifactQuota = ArtifactQuota {
        max_files: 100,
        max_bytes: 1 << 20,
    };

    fn session(runner: Arc<PageRunner>, decoder: Arc<RecordingDecoder>) -> SaneDeviceSession {
        SaneDeviceSession::new_with_adapters(
            "sane:test".into(),
            "test".into(),
            false,
            runner,
            decoder,
        )
    }

    fn stream(
        session: &SaneDeviceSession,
        output: &TemporaryOutput,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        let spec = CommandSpec {
            program: "scanimage".into(),
            args: Vec::new(),
        };
        session.stream_scanimage_pages(&spec, output, QUOTA, max_pages, emit)
    }

    fn output_dir() -> TemporaryOutput {
        TemporaryOutput::new("sane-test", "png").unwrap()
    }

    #[test]
    fn batch_pages_stream_in_order_and_are_removed_once_emitted() {
        let decoder = Arc::new(RecordingDecoder::default());
        let session = session(
            PageRunner::new(3, Some(output(true, Some(0), ""))),
            decoder.clone(),
        );
        let output = output_dir();
        let mut emitted = 0;
        let result = stream(&session, &output, 5, &mut |_| {
            emitted += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(result, ScanPagesResult::feeder_exhausted(3));
        assert_eq!(emitted, 3);
        assert_eq!(
            *decoder.0.lock().unwrap(),
            ["page_000001.png", "page_000002.png", "page_000003.png"]
        );
        assert_eq!(std::fs::read_dir(output.directory()).unwrap().count(), 0);
    }

    #[test]
    fn cancel_during_a_batch_keeps_the_emitted_pages() {
        let runner = PageRunner::new(3, None);
        let session = session(runner.clone(), Arc::default());
        let output = output_dir();
        let mut emitted = 0;
        let result = stream(&session, &output, 5, &mut |_| {
            emitted += 1;
            if emitted == 2 {
                session.cancel();
            }
            Ok(())
        });
        assert!(matches!(result, Err(ScanError::Cancelled(_))));
        assert_eq!(emitted, 2);
        assert!(runner.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn failed_emit_stops_the_batch_and_returns_its_error() {
        let runner = PageRunner::new(3, None);
        let session = session(runner.clone(), Arc::default());
        let output = output_dir();
        let mut calls = 0;
        let result = stream(&session, &output, 5, &mut |_| {
            calls += 1;
            if calls == 2 {
                return Err(ScanError::Other("publish failed".into()));
            }
            Ok(())
        });
        assert!(matches!(result, Err(ScanError::Other(message)) if message == "publish failed"));
        assert_eq!(calls, 2);
        assert!(runner.stopped.load(Ordering::SeqCst));
        assert!(!session.session.is_cancelled());
    }

    #[test]
    fn more_pages_than_requested_stops_the_batch() {
        let runner = PageRunner::new(3, None);
        let session = session(runner.clone(), Arc::default());
        let output = output_dir();
        let mut emitted = 0;
        let result = stream(&session, &output, 2, &mut |_| {
            emitted += 1;
            Ok(())
        });
        assert!(matches!(result, Err(ScanError::Other(message)) if message.contains("beyond")));
        assert!(emitted <= 2);
        assert!(runner.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn failed_batch_still_emits_scanned_pages_then_returns_the_error() {
        let failure = output(
            false,
            Some(9),
            "scanimage: sane_start: Document feeder jammed",
        );
        let session = session(PageRunner::new(2, Some(failure)), Arc::default());
        let output = output_dir();
        let mut emitted = 0;
        let result = stream(&session, &output, 5, &mut |_| {
            emitted += 1;
            Ok(())
        });
        assert_eq!(emitted, 2);
        assert!(
            matches!(result, Err(ScanError::Unsupported(message)) if message.contains("jammed"))
        );
    }

    #[test]
    fn failed_batch_without_pages_returns_the_original_error() {
        let failure = output(false, Some(9), "cover open");
        let session = session(PageRunner::new(0, Some(failure)), Arc::default());
        let output = output_dir();
        let result = stream(&session, &output, 5, &mut |_| panic!("no page to emit"));
        assert!(matches!(result, Err(ScanError::Unsupported(_))));
    }

    #[test]
    fn emptied_feeder_batch_reports_exhaustion_after_its_pages() {
        let session = session(
            PageRunner::new(2, Some(output(false, Some(7), ""))),
            Arc::default(),
        );
        let output = output_dir();
        let result = stream(&session, &output, 5, &mut |_| Ok(())).unwrap();
        assert_eq!(result.emitted, 2);
    }

    fn output(success: bool, exit_code: Option<i32>, stderr: &str) -> CommandOutput {
        CommandOutput {
            success,
            exit_code,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn emptied_feeder_is_recognized_by_exit_status_in_any_language() {
        let german = "scanimage: sane_start: Dokumenteneinzug hat keine Dokumente mehr";
        assert!(validate_batch_command(&output(false, Some(7), german)).is_ok());
        assert!(validate_batch_command(&output(false, Some(9), german)).is_err());
        assert!(validate_batch_command(&output(
            false,
            Some(1),
            "scanimage: sane_start: Document feeder out of documents"
        ))
        .is_ok());
    }

    /// Answers `scanimage --help` and records each call's arguments.
    #[derive(Default)]
    struct HelpRunner {
        calls: Mutex<Vec<Vec<String>>>,
        /// The default source lists page-width as active; reads with
        /// `--source=` fail.
        active_default_failing_reads: bool,
    }

    impl CommandRunner for HelpRunner {
        fn run(
            &self,
            spec: &CommandSpec,
            _: Duration,
            _: &Mutex<bool>,
            _: Option<&CancellationToken>,
            artifacts: Option<ArtifactWatch<'_>>,
        ) -> Result<CommandOutput> {
            assert!(artifacts.is_none(), "only the help probe runs");
            self.calls.lock().unwrap().push(spec.args.clone());
            let duplex = spec.args.iter().any(|arg| arg == "--source=ADF Duplex");
            if self.active_default_failing_reads {
                if duplex {
                    return Err(ScanError::Unsupported("probe failed".into()));
                }
                return Ok(CommandOutput {
                    success: true,
                    exit_code: Some(0),
                    stdout: b"    --source ADF Front|ADF Duplex [ADF Front]\n    --page-width 0..215.9mm [215.9]\n"
                        .to_vec(),
                    stderr: Vec::new(),
                });
            }
            let help = if duplex {
                "    --source ADF Front|ADF Duplex [ADF Duplex]\n    --page-width 0..215.9mm [215.9]\n"
            } else {
                "    --source ADF Front|ADF Duplex [ADF Front]\n    --page-width 0..215.9mm [215.9] [inactive]\n"
            };
            Ok(CommandOutput {
                success: true,
                exit_code: Some(0),
                stdout: help.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn help_is_read_again_for_a_source_other_than_the_default() {
        use crate::domain::acquisition::ScanRequest;
        let runner = Arc::new(HelpRunner::default());
        let session = SaneDeviceSession::new_with_adapters(
            "sane:test".into(),
            "test".into(),
            false,
            runner.clone(),
            Arc::new(RecordingDecoder::default()),
        );
        let binary = Path::new("scanimage");
        let base = session.read_help(binary, None).unwrap();
        assert!(base.source_dependent_options && !base.page_width_option);
        assert_eq!(runner.calls.lock().unwrap().len(), 1);

        // The default source needs no second read.
        let simplex = ScanRequest {
            mode: crate::domain::acquisition::ScanMode::Document,
            ..ScanRequest::default()
        };
        assert!(
            !session
                .capabilities_for_source(binary, &base, &simplex)
                .page_width_option
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 1);

        // A duplex request selects another source: one more read, then cached.
        let duplex = ScanRequest {
            duplex: true,
            ..simplex
        };
        for _ in 0..2 {
            assert!(
                session
                    .capabilities_for_source(binary, &base, &duplex)
                    .page_width_option
            );
        }
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1],
            ["--help", "-d", "test", "--source=ADF Duplex"].map(String::from)
        );
    }

    #[test]
    fn an_active_option_and_a_failed_re_read_are_handled() {
        use crate::domain::acquisition::{ScanMode, ScanRequest};
        let runner = Arc::new(HelpRunner {
            active_default_failing_reads: true,
            ..HelpRunner::default()
        });
        let session = SaneDeviceSession::new_with_adapters(
            "sane:test".into(),
            "test".into(),
            false,
            runner.clone(),
            Arc::new(RecordingDecoder::default()),
        );
        let binary = Path::new("scanimage");
        let base = session.read_help(binary, None).unwrap();
        assert!(base.source_dependent_options && base.page_width_option);
        let duplex = ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            ..ScanRequest::default()
        };
        // The other source is probed once even though the option is active;
        // the failure is cached, so later scans do not probe again.
        for _ in 0..2 {
            assert_eq!(
                session.capabilities_for_source(binary, &base, &duplex),
                base
            );
        }
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn open_lists_again_only_after_a_cache_hit_that_misses() {
        use crate::infrastructure::acquisition::DeviceInfo;
        let device = |id: &str| DeviceInfo::new(id, "scanner", "sane");
        let relist = || vec![device("sane:a")];
        // A reused listing without the id may predate a replug: list again.
        let found = super::list_for_open("sane:a", || (Vec::new(), true), relist);
        assert_eq!(found.len(), 1);
        // A fresh listing is already current: no second discovery.
        let fresh = super::list_for_open(
            "sane:a",
            || (Vec::new(), false),
            || panic!("must not list again"),
        );
        assert!(fresh.is_empty());
        // A cache hit that has the id is used as is.
        let hit = super::list_for_open(
            "sane:a",
            || (vec![device("sane:a")], true),
            || panic!("must not list again"),
        );
        assert_eq!(hit.len(), 1);
    }
}
