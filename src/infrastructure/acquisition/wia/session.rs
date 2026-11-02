use super::command::{feeder_exhausted, wia_transfer_command, wia_transfer_pages_command};
use super::probing::SystemCommandRunner;
use crate::domain::acquisition::reject_single_page_duplex;
use crate::domain::acquisition::{
    apply_flat_dark_cal, synthetic_cal_tables, validate_scan_dpi, ScanMode, ScanRequest,
};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::batch::batch_timeout;
use crate::infrastructure::acquisition::command_backend::numbered_page_outputs;
use crate::infrastructure::acquisition::{
    opens_simulated_session, simulate_backends, DeviceInfo, DeviceMaintenanceCapabilities,
    DeviceSession, ScanPagesResult,
};
use crate::infrastructure::media::NativeImageDecoder;
use crate::infrastructure::runtime::{
    artifact_quota_for_request, validate_artifact_quota, ArtifactWatch, CommandOutput,
    CommandRunner, CommandSession, ImageDecoder, TemporaryOutput,
};
use crate::operation::CancellationToken;
use std::sync::{Arc, Mutex};

/// Full WIA session: open succeeds for listed devices; scan attempts transfer.
pub struct WiaDeviceSession {
    pub device_id: String,
    /// Raw WIA DeviceID (without wia: prefix)
    raw_id: String,
    simulate: bool,
    session: CommandSession,
    cal: Mutex<Option<CalTables>>,
    focus: Mutex<Option<(f64, f64)>>,
    runner: Arc<dyn CommandRunner>,
    decoder: Arc<dyn ImageDecoder>,
}

#[derive(Clone)]
struct CalTables {
    dark: Vec<i32>,
    flat: Vec<f64>,
}

impl WiaDeviceSession {
    pub fn new(device_id: String, raw_id: String, simulate: bool) -> Self {
        Self {
            device_id,
            raw_id,
            simulate,
            session: CommandSession::default(),
            cal: Mutex::new(None),
            focus: Mutex::new(None),
            runner: Arc::new(SystemCommandRunner),
            decoder: Arc::new(NativeImageDecoder),
        }
    }

    pub fn new_with_adapters(
        device_id: String,
        raw_id: String,
        simulate: bool,
        runner: Arc<dyn CommandRunner>,
        decoder: Arc<dyn ImageDecoder>,
    ) -> Self {
        Self {
            device_id,
            raw_id,
            simulate,
            session: CommandSession::default(),
            cal: Mutex::new(None),
            focus: Mutex::new(None),
            runner,
            decoder,
        }
    }

    /// Acquire via PowerShell COM Transfer → temp PNG → load_image.
    fn acquire_com(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let output_file = TemporaryOutput::new("wia", "png")?;
        let artifact_quota = artifact_quota_for_request(request, 1)?;
        let spec = wia_transfer_command(&self.raw_id, request, output_file.path());
        super::super::batch::run_materialized_scan_command(
            &self.session,
            self.runner.as_ref(),
            self.decoder.as_ref(),
            &spec,
            &output_file,
            artifact_quota,
            "WIA acquire failed",
        )
    }

    fn acquire_com_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        let (output, artifact_quota, command_output) = self.run_com_page_job(request, max_pages)?;
        let command_output = match command_output {
            Ok(command_output) => command_output,
            Err(error @ ScanError::Cancelled(_)) => return Err(error),
            // A timeout or quota stop still leaves the finished pages on disk.
            Err(error) => {
                let mut paths =
                    numbered_page_outputs(output.directory(), "png").unwrap_or_default();
                paths.truncate(max_pages as usize);
                self.salvage_materialized_pages(paths, emit);
                return Err(error);
            }
        };
        self.finish_com_pages(&output, artifact_quota, &command_output, max_pages, emit)
    }

    /// Emit the pages a finished WIA batch left in `output`. When the command
    /// failed (jam, open cover, USB error) the pages already scanned are still
    /// emitted, then the command's error is returned.
    fn finish_com_pages(
        &self,
        output: &TemporaryOutput,
        artifact_quota: crate::infrastructure::runtime::ArtifactQuota,
        command_output: &CommandOutput,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        validate_artifact_quota(output.directory(), artifact_quota)?;
        let mut paths = numbered_page_outputs(output.directory(), "png")?;
        if let Err(failure) = validate_batch_output(&self.session, command_output) {
            if matches!(failure, ScanError::Cancelled(_)) {
                return Err(failure);
            }
            paths.truncate(max_pages as usize);
            self.salvage_materialized_pages(paths, emit);
            return Err(failure);
        }
        super::super::command_backend::validate_batch_page_count(paths.len(), max_pages, "WIA")?;
        let emitted = self.emit_materialized_pages(paths, emit)?;
        Ok(super::super::command_backend::command_backed_pages_result(
            emitted, max_pages,
        ))
    }

    fn run_com_page_job(
        &self,
        request: &ScanRequest,
        max_pages: u32,
    ) -> Result<(
        TemporaryOutput,
        crate::infrastructure::runtime::ArtifactQuota,
        Result<CommandOutput>,
    )> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let output = TemporaryOutput::new("wia-pages", "png")?;
        let artifact_quota = artifact_quota_for_request(request, max_pages)?;
        let command = self.run_pages_command(request, max_pages, &output, artifact_quota);
        Ok((output, artifact_quota, command))
    }

    fn run_pages_command(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        output: &TemporaryOutput,
        artifact_quota: crate::infrastructure::runtime::ArtifactQuota,
    ) -> Result<CommandOutput> {
        let spec = wia_transfer_pages_command(&self.raw_id, request, output.directory(), max_pages);
        self.runner.run(
            &spec,
            batch_timeout(max_pages),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            Some(ArtifactWatch {
                directory: output.directory(),
                quota: artifact_quota,
            }),
        )
    }

    /// Emit the pages of a failed batch in order, stopping at the first page
    /// that cannot be decoded or published (typically the interrupted one).
    /// The caller reports the batch failure itself, as the SANE adapter does.
    fn salvage_materialized_pages(
        &self,
        paths: Vec<std::path::PathBuf>,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) {
        for path in paths {
            if self.emit_materialized_pages(vec![path], emit).is_err() {
                break;
            }
        }
    }

    fn emit_materialized_pages(
        &self,
        paths: Vec<std::path::PathBuf>,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<u32> {
        let successful_output = CommandOutput {
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
                &successful_output,
                &path,
                self.decoder.as_ref(),
                "WIA batch acquire failed",
            )?;
            emit(self.calibrated_image(image)?)?;
            emitted += 1;
        }
        Ok(emitted)
    }

    fn sim_gradient(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        super::super::mock::MockDeviceSession::gradient(request)
    }

    fn calibrated_image(&self, image: ImageBuffer) -> Result<ImageBuffer> {
        let Some(cal) = self.cal.lock().ok().and_then(|tables| tables.clone()) else {
            return Ok(image);
        };
        apply_flat_dark_cal(&image, &cal.dark, &cal.flat)
    }

    fn scan_simulated_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        for index in 0..max_pages {
            if self.session.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            let mut page_request = request.clone();
            page_request.duplex = false;
            page_request.seed = request.seed.saturating_add(index);
            emit(self.calibrated_image(self.sim_gradient(&page_request)?)?)?;
        }
        Ok(ScanPagesResult::limit_reached(max_pages))
    }
}

fn validate_batch_output(session: &CommandSession, output: &CommandOutput) -> Result<()> {
    if session.is_cancelled() {
        return Err(ScanError::Cancelled("scan cancelled".into()));
    }
    if output.success || feeder_exhausted(output) {
        return Ok(());
    }
    Err(ScanError::Unsupported(format!(
        "WIA batch acquire failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

impl DeviceSession for WiaDeviceSession {
    fn maintenance_capabilities(&self) -> DeviceMaintenanceCapabilities {
        if self.simulate {
            DeviceMaintenanceCapabilities::simulated_point_focus()
        } else {
            DeviceMaintenanceCapabilities::unsupported(
                "WIA maintenance is not portable; camera focus is never used",
            )
        }
    }

    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        super::super::batch::validate_session_state(
            self.session.is_closed(),
            self.session.is_cancelled(),
        )?;
        let img = if self.simulate {
            self.sim_gradient(request)?
        } else {
            self.acquire_com(request)?
        };
        self.calibrated_image(img)
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
        match self.simulate {
            true => self.scan_simulated_pages(request, max_pages, emit),
            false => self.scan_hardware_pages(request, max_pages, emit),
        }
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
        use serde_json::json;
        if self.session.is_closed() {
            return crate::infrastructure::acquisition::contract::closed_session_envelope("wia");
        }
        // Hardware WIA has no portable calibrate; sim stores tables like mock.
        if self.simulate {
            let (dark, flat) = synthetic_cal_tables();
            if let Ok(mut g) = self.cal.lock() {
                *g = Some(CalTables { dark, flat });
            }
            return json!({
                "ok": true,
                "status": "calibrated",
                "backend": "wia",
                "device_id": self.device_id,
            });
        }
        json!({
            "ok": false,
            "status": "unsupported",
            "backend": "wia",
            "device_id": self.device_id,
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        use serde_json::json;
        let (xf, yf, value) =
            crate::infrastructure::acquisition::contract::simulated_focus_score(x, y);
        if self.simulate {
            if let Ok(mut g) = self.focus.lock() {
                *g = Some((xf, yf));
            }
            return json!({
                "ok": true,
                "status": "focused",
                "x_frac": xf,
                "y_frac": yf,
                "focus": value,
                "backend": "wia",
            });
        }
        json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": xf,
            "y_frac": yf,
            "focus": 0.0,
            "backend": "wia",
        })
    }
}

impl WiaDeviceSession {
    fn scan_hardware_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        if request.mode == ScanMode::Document {
            return self.acquire_com_pages(request, max_pages, emit);
        }
        if request.duplex {
            return Err(ScanError::Unsupported(
                "WIA duplex acquisition requires an ADF source".into(),
            ));
        }
        for index in 0..max_pages {
            let mut page_request = request.clone();
            page_request.seed = request.seed.saturating_add(index);
            emit(self.acquire_com(&page_request)?)?;
        }
        Ok(ScanPagesResult::limit_reached(max_pages))
    }
}

/// Open a WIA session for a listed device (or sim). Does **not** permanently refuse open.
pub fn open(device_id: &str) -> Result<WiaDeviceSession> {
    open_with_cancellation(device_id, None)
}

pub fn open_with_cancellation(
    device_id: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<WiaDeviceSession> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    let id = device_id.trim();
    if !id.starts_with("wia:") && id != "wia" {
        return Err(ScanError::DeviceNotFound(format!(
            "not a WIA device id: {id}"
        )));
    }
    if opens_simulated_session(id, "wia:sim", simulate_backends()) {
        return Ok(WiaDeviceSession::new(id.into(), "sim".into(), true));
    }
    let listed = list_for_open(
        id,
        || super::probing::list_devices_with_cancellation(cancellation),
        // With a token the first listing was already uncached.
        cancellation
            .is_none()
            .then_some(super::probing::list_devices_refreshing_cache),
    );
    if !listed.iter().any(|d| d.id == id) {
        return Err(ScanError::DeviceNotFound(format!(
            "unknown WIA device: {id}"
        )));
    }
    let raw = id.strip_prefix("wia:").unwrap_or(id).to_string();
    Ok(WiaDeviceSession::new(id.into(), raw, false))
}

/// Devices to check `id` against. The first listing may be a cached one that
/// predates the scanner being switched on, so a miss lists once more.
fn list_for_open(
    id: &str,
    first: impl FnOnce() -> Vec<DeviceInfo>,
    retry: Option<impl FnOnce() -> Vec<DeviceInfo>>,
) -> Vec<DeviceInfo> {
    let listed = first();
    match retry {
        Some(retry) if !listed.iter().any(|device| device.id == id) => retry(),
        _ => listed,
    }
}

#[cfg(test)]
mod tests {
    use super::{list_for_open, WiaDeviceSession};
    use crate::domain::image::{ImageBuffer, PixelFormat};
    use crate::error::{Result, ScanError};
    use crate::infrastructure::acquisition::DeviceInfo;
    use crate::infrastructure::runtime::{
        ArtifactQuota, ArtifactWatch, CommandOutput, CommandRunner, CommandSpec, ImageDecoder,
        TemporaryOutput,
    };
    use crate::operation::CancellationToken;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct UnusedRunner;

    impl CommandRunner for UnusedRunner {
        fn run(
            &self,
            _: &CommandSpec,
            _: Duration,
            _: &Mutex<bool>,
            _: Option<&CancellationToken>,
            _: Option<ArtifactWatch<'_>>,
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

    fn session() -> WiaDeviceSession {
        WiaDeviceSession::new_with_adapters(
            "wia:test".into(),
            "test".into(),
            false,
            Arc::new(UnusedRunner),
            Arc::new(PixelDecoder),
        )
    }

    fn output_with_pages(count: u32) -> TemporaryOutput {
        let output = TemporaryOutput::new("wia-test", "png").unwrap();
        for number in 1..=count {
            std::fs::write(
                output.directory().join(format!("page_{number:06}.png")),
                b"x",
            )
            .unwrap();
        }
        output
    }

    fn failed(stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            exit_code: Some(1),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn failed_batch_still_emits_scanned_pages_then_returns_the_error() {
        let output = output_with_pages(2);
        let mut emitted = 0;
        let result =
            session().finish_com_pages(&output, QUOTA, &failed("paper jam"), 5, &mut |_| {
                emitted += 1;
                Ok(())
            });
        assert_eq!(emitted, 2);
        assert!(
            matches!(result, Err(ScanError::Unsupported(message)) if message.contains("paper jam"))
        );
    }

    #[test]
    fn emptied_feeder_batch_reports_exhaustion_after_its_pages() {
        let output = output_with_pages(2);
        let result = session()
            .finish_com_pages(
                &output,
                QUOTA,
                &failed("OSL_FEEDER_EXHAUSTED"),
                5,
                &mut |_| Ok(()),
            )
            .unwrap();
        assert_eq!(result.emitted, 2);
    }

    #[test]
    fn a_missing_device_is_looked_up_again_uncached() {
        let device = |id: &str| DeviceInfo::new(id, "scanner", "wia");
        let found = list_for_open("wia:a", Vec::new, Some(|| vec![device("wia:a")]));
        assert_eq!(found.len(), 1);
        // A hit, or a listing that was already uncached, does not list again.
        let hit = list_for_open(
            "wia:a",
            || vec![device("wia:a")],
            Some(|| panic!("must not list again")),
        );
        assert_eq!(hit.len(), 1);
        let none = list_for_open("wia:a", Vec::new, None::<fn() -> Vec<DeviceInfo>>);
        assert!(none.is_empty());
    }
}
