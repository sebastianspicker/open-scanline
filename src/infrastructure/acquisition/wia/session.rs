use super::command::{
    feeder_exhausted, numbered_page_outputs, wia_transfer_command, wia_transfer_pages_command,
};
use super::probing::SystemCommandRunner;
use crate::domain::acquisition::{
    apply_flat_dark_cal, synthetic_cal_tables, validate_scan_dpi, ScanMode, ScanRequest,
};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{
    artifact_quota_for_request, document_batch_timeout, simulate_backends, validate_artifact_quota,
    CommandOutput, CommandRunner, CommandSession, ImageDecoder, NativeImageDecoder,
    TemporaryOutput,
};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::acquisition::{
    reject_single_page_duplex, DeviceMaintenanceCapabilities, DeviceSession, ScanPagesResult,
};
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
            request,
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
        validate_artifact_quota(output.directory(), artifact_quota)?;
        validate_batch_output(&self.session, &command_output)?;
        let paths = numbered_page_outputs(output.directory())?;
        validate_page_count(paths.len(), max_pages)?;
        let emitted = self.emit_materialized_pages(paths, request, emit)?;
        Ok(batch_result(emitted, max_pages))
    }

    fn run_com_page_job(
        &self,
        request: &ScanRequest,
        max_pages: u32,
    ) -> Result<(
        TemporaryOutput,
        crate::infrastructure::runtime::ArtifactQuota,
        CommandOutput,
    )> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let output = TemporaryOutput::new("wia-pages", "png")?;
        let artifact_quota = artifact_quota_for_request(request, max_pages)?;
        let command = self.run_pages_command(request, max_pages, &output, artifact_quota)?;
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
        self.runner.run_with_cancellation_and_artifact_quota(
            &spec,
            document_batch_timeout(max_pages),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            output.directory(),
            artifact_quota,
        )
    }

    fn emit_materialized_pages(
        &self,
        paths: Vec<std::path::PathBuf>,
        request: &ScanRequest,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<u32> {
        let successful_output = CommandOutput {
            success: true,
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
                request,
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

fn validate_page_count(actual: usize, maximum: u32) -> Result<()> {
    if actual <= maximum as usize {
        return Ok(());
    }
    Err(ScanError::Other(format!(
        "WIA emitted {actual} pages beyond the requested limit {maximum}"
    )))
}

fn batch_result(emitted: u32, maximum: u32) -> ScanPagesResult {
    if emitted == maximum {
        ScanPagesResult::limit_reached(emitted)
    } else {
        ScanPagesResult::feeder_exhausted(emitted)
    }
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
            return json!({"ok": false, "status": "closed", "backend": "wia"});
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
        let xf = x.clamp(0.0, 1.0);
        let yf = y.clamp(0.0, 1.0);
        if self.simulate {
            if let Ok(mut g) = self.focus.lock() {
                *g = Some((xf, yf));
            }
            // Match mock behavior: ((1.0 - min(1.0, dist*1.4)) * 10000).round() / 10000
            let dist = ((xf - 0.5).powi(2) + (yf - 0.5).powi(2)).sqrt();
            let value = ((1.0 - (dist * 1.4).min(1.0)) * 10000.0).round() / 10000.0;
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
    if id == "wia:sim" || (simulate_backends() && id.ends_with(":sim")) {
        return Ok(WiaDeviceSession::new(id.into(), "sim".into(), true));
    }
    let listed = super::probing::list_devices_with_cancellation(cancellation);
    if !listed.iter().any(|d| d.id == id) {
        return Err(ScanError::DeviceNotFound(format!(
            "unknown WIA device: {id}"
        )));
    }
    let raw = id.strip_prefix("wia:").unwrap_or(id).to_string();
    Ok(WiaDeviceSession::new(id.into(), raw, false))
}
