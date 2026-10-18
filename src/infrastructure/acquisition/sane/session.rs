use super::capabilities::{
    parse_scanimage_help, parse_scanimage_maintenance_options, scanimage_maintenance_command,
    scanimage_maintenance_inspection_command, SaneCapabilities, SaneMaintenanceAction,
    SaneMaintenanceOptions,
};
use super::command::build_scanimage_command;
use super::probing::{which, SystemCommandRunner};
use super::{emit_simulated_pages, emit_single_pages};
use crate::domain::acquisition::reject_single_page_duplex;
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::batch::{batch_timeout, probe_timeout};
use crate::infrastructure::acquisition::{
    opens_simulated_session, simulate_backends, DeviceMaintenanceCapabilities, DeviceSession,
    ScanPagesResult,
};
use crate::infrastructure::media::NativeImageDecoder;
use crate::infrastructure::runtime::{
    artifact_quota_for_request, validate_artifact_quota, CommandOutput, CommandRunner,
    CommandSession, CommandSpec, ImageDecoder, TemporaryOutput,
};
use crate::operation::CancellationToken;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

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
            maintenance_options: OnceLock::new(),
        }
    }

    /// Capabilities for a real acquisition. Without readable device options no
    /// source, colour mode, or duplex value can be chosen safely, so a session
    /// that inspects its device refuses to guess them.
    fn acquisition_capabilities(&self, binary: &Path) -> Result<Option<&SaneCapabilities>> {
        let capabilities = self.capabilities(binary);
        if capabilities.is_none() && self.inspect_capabilities {
            return Err(ScanError::Unsupported(format!(
                "could not read the options of SANE device {name}: `scanimage --help -d {name}` \
                 failed or listed none; run it to check the device, or rerun with --verbose",
                name = self.sane_name
            )));
        }
        Ok(capabilities)
    }

    fn capabilities(&self, binary: &Path) -> Option<&SaneCapabilities> {
        self.capabilities
            .get_or_init(|| {
                if !self.inspect_capabilities {
                    return None;
                }
                let spec = CommandSpec {
                    program: binary.display().to_string(),
                    args: vec!["--help".into(), "-d".into(), self.sane_name.clone()],
                };
                let output = self
                    .runner
                    .run_with_cancellation(
                        &spec,
                        probe_timeout(Duration::from_secs(8)),
                        self.session.cancelled(),
                        self.session.cancellation_token().as_ref(),
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
            })
            .as_ref()
    }

    fn maintenance_options(&self, binary: &Path) -> Option<&SaneMaintenanceOptions> {
        self.maintenance_options
            .get_or_init(|| {
                let spec = scanimage_maintenance_inspection_command(binary, &self.sane_name);
                let output = self
                    .runner
                    .run_with_cancellation(
                        &spec,
                        probe_timeout(Duration::from_secs(8)),
                        self.session.cancelled(),
                        self.session.cancellation_token().as_ref(),
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
            self.runner.run_with_cancellation(
                &spec,
                Duration::from_secs(20),
                self.session.cancelled(),
                self.session.cancellation_token().as_ref(),
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
            self.acquisition_capabilities(&bin)?,
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
        let (output, artifact_quota, command) = self.run_scanimage_page_job(request, max_pages)?;
        let command = match command {
            Ok(command) => command,
            Err(error @ ScanError::Cancelled(_)) => return Err(error),
            // A timeout or quota stop still leaves the finished pages on disk.
            Err(error) => {
                let mut paths = numbered_batch_outputs(output.directory()).unwrap_or_default();
                paths.truncate(max_pages as usize);
                self.salvage_batch_pages(paths, emit);
                return Err(error);
            }
        };
        self.finish_scanimage_pages(&output, artifact_quota, &command, max_pages, emit)
    }

    /// Emit the pages a finished `scanimage` batch left in `output`. When the
    /// command failed (jam, open cover, USB error) the pages already scanned
    /// are still emitted, then the command's error is returned.
    fn finish_scanimage_pages(
        &self,
        output: &TemporaryOutput,
        artifact_quota: crate::infrastructure::runtime::ArtifactQuota,
        command: &CommandOutput,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        validate_artifact_quota(output.directory(), artifact_quota)?;
        let mut paths = numbered_batch_outputs(output.directory())?;
        if let Err(failure) = validate_batch_command(command) {
            paths.truncate(max_pages as usize);
            self.salvage_batch_pages(paths, emit);
            return Err(failure);
        }
        super::super::command_backend::validate_batch_page_count(
            paths.len(),
            max_pages,
            "scanimage",
        )?;
        let emitted = self.emit_batch_pages(paths, emit)?;
        Ok(super::super::command_backend::command_backed_pages_result(
            emitted, max_pages,
        ))
    }

    fn run_scanimage_page_job(
        &self,
        request: &ScanRequest,
        max_pages: u32,
    ) -> Result<(
        TemporaryOutput,
        crate::infrastructure::runtime::ArtifactQuota,
        Result<CommandOutput>,
    )> {
        let binary = which("scanimage")
            .ok_or_else(|| ScanError::Unsupported("SANE scan requires scanimage CLI".into()))?;
        let output = TemporaryOutput::new("sane-batch", "png")?;
        let quota = artifact_quota_for_request(request, max_pages)?;
        let spec = self.batch_scanimage_command(&binary, request, &output, max_pages)?;
        let command = self.runner.run_with_cancellation_and_artifact_quota(
            &spec,
            batch_timeout(max_pages),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            output.directory(),
            quota,
        );
        Ok((output, quota, command))
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
            self.acquisition_capabilities(binary)?,
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

    /// Emit the pages of a failed batch in order, stopping at the first page
    /// that cannot be decoded or published (typically the interrupted one).
    /// The caller reports the batch failure itself.
    fn salvage_batch_pages(
        &self,
        paths: Vec<std::path::PathBuf>,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) {
        for path in paths {
            if self.emit_batch_pages(vec![path], emit).is_err() {
                break;
            }
        }
    }

    fn emit_batch_pages(
        &self,
        paths: Vec<std::path::PathBuf>,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<u32> {
        let success = CommandOutput {
            success: true,
            exit_code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let mut emitted = 0_u32;
        for path in paths {
            if self.session.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            let image = self.session.decode_materialized_output(
                &success,
                &path,
                self.decoder.as_ref(),
                "scanimage batch failed",
            )?;
            emit(image)?;
            emitted += 1;
        }
        Ok(emitted)
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

fn validate_batch_command(command: &CommandOutput) -> Result<()> {
    if command.success
        || command.exit_code == Some(SANE_STATUS_NO_DOCS)
        || feeder_exhausted(&command.stderr)
    {
        return Ok(());
    }
    Err(ScanError::Unsupported(format!(
        "scanimage batch failed: {}",
        String::from_utf8_lossy(&command.stderr).trim()
    )))
}

fn numbered_batch_outputs(directory: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("page_") && name.ends_with(".png"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn feeder_exhausted(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    [
        "out of documents",
        "no documents",
        "document feeder empty",
        "paper empty",
        "no paper",
    ]
    .iter()
    .any(|message| stderr.contains(message))
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
    let mut listed = super::probing::recent_devices(cancellation);
    if !listed.iter().any(|d| d.id == id) {
        // The reused listing may predate a replug; USB names change with it.
        listed = super::probing::list_devices_with_cancellation(cancellation);
    }
    if !listed.iter().any(|d| d.id == id) {
        return Err(unknown_device(id, &listed));
    }
    let name = id.strip_prefix("sane:").unwrap_or(id).to_string();
    Ok(SaneDeviceSession::new(id.into(), name, false))
}

fn unknown_device(
    id: &str,
    listed: &[crate::infrastructure::acquisition::DeviceInfo],
) -> ScanError {
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
    use crate::infrastructure::runtime::{
        ArtifactQuota, CommandRunner, CommandSpec, ImageDecoder, TemporaryOutput,
    };
    use crate::operation::CancellationToken;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct UnusedRunner;

    impl CommandRunner for UnusedRunner {
        fn run(&self, _: &CommandSpec, _: Duration, _: &Mutex<bool>) -> Result<CommandOutput> {
            unreachable!("page emission does not run commands")
        }

        fn run_with_cancellation(
            &self,
            _: &CommandSpec,
            _: Duration,
            _: &Mutex<bool>,
            _: Option<&CancellationToken>,
        ) -> Result<CommandOutput> {
            unreachable!("page emission does not run commands")
        }

        fn run_with_cancellation_and_artifact_quota(
            &self,
            _: &CommandSpec,
            _: Duration,
            _: &Mutex<bool>,
            _: Option<&CancellationToken>,
            _: &Path,
            _: ArtifactQuota,
        ) -> Result<CommandOutput> {
            unreachable!("page emission does not run commands")
        }
    }

    struct PixelDecoder;

    impl ImageDecoder for PixelDecoder {
        fn decode(&self, _: &Path) -> Result<ImageBuffer> {
            ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![0, 0, 0])
        }
    }

    const QUOTA: ArtifactQuota = ArtifactQuota {
        max_files: 100,
        max_bytes: 1 << 20,
    };

    fn session() -> SaneDeviceSession {
        SaneDeviceSession::new_with_adapters(
            "sane:test".into(),
            "test".into(),
            false,
            Arc::new(UnusedRunner),
            Arc::new(PixelDecoder),
        )
    }

    fn output_with_pages(count: u32) -> TemporaryOutput {
        let output = TemporaryOutput::new("sane-test", "png").unwrap();
        for number in 1..=count {
            std::fs::write(
                output.directory().join(format!("page_{number:06}.png")),
                b"x",
            )
            .unwrap();
        }
        output
    }

    #[test]
    fn failed_batch_still_emits_scanned_pages_then_returns_the_error() {
        let output = output_with_pages(2);
        let mut emitted = 0;
        let result = session().finish_scanimage_pages(
            &output,
            QUOTA,
            &self::output(
                false,
                Some(9),
                "scanimage: sane_start: Document feeder jammed",
            ),
            5,
            &mut |_| {
                emitted += 1;
                Ok(())
            },
        );
        assert_eq!(emitted, 2);
        assert!(
            matches!(result, Err(ScanError::Unsupported(message)) if message.contains("jammed"))
        );
    }

    #[test]
    fn failed_batch_without_pages_returns_the_original_error() {
        let output = output_with_pages(0);
        let result = session().finish_scanimage_pages(
            &output,
            QUOTA,
            &self::output(false, Some(9), "cover open"),
            5,
            &mut |_| panic!("no page to emit"),
        );
        assert!(matches!(result, Err(ScanError::Unsupported(_))));
    }

    #[test]
    fn emptied_feeder_batch_reports_exhaustion_after_its_pages() {
        let output = output_with_pages(2);
        let result = session()
            .finish_scanimage_pages(
                &output,
                QUOTA,
                &self::output(false, Some(7), ""),
                5,
                &mut |_| Ok(()),
            )
            .unwrap();
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
}
