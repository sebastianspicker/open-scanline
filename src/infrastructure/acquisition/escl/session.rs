use super::*;

pub struct EsclDeviceSession {
    pub device_id: String,
    pub(crate) endpoint: Endpoint,
    pub(crate) simulate: bool,
    pub(crate) closed: Mutex<bool>,
    pub(crate) cancelled: Mutex<bool>,
    pub(crate) cancellation: Mutex<Option<CancellationToken>>,
}

impl EsclDeviceSession {
    pub(crate) fn new(device_id: String, endpoint: Endpoint, simulate: bool) -> Self {
        Self {
            device_id,
            endpoint,
            simulate,
            closed: Mutex::new(false),
            cancelled: Mutex::new(false),
            cancellation: Mutex::new(Some(CancellationToken::new())),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        *self.closed.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner())
            || self
                .cancellation
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .unwrap_or_default()
    }

    pub(crate) fn create_job(&self, root: &str, settings: &[u8]) -> Result<ScanJob> {
        let path = format!("/{root}/ScanJobs");
        let cancellation = self.cancellation_token();
        let (status, body, headers) = http_exchange_with_cancellation_result(
            &self.endpoint,
            "POST",
            &path,
            HttpExchangeOptions {
                body: Some(settings),
                content_type: Some("text/xml"),
                global_timeout: Duration::from_secs(15),
                setup_timeout: CONTROL_REQUEST_SETUP_TIMEOUT,
                response_header_timeout: Duration::from_secs(15),
                response_limit: JOB_RESPONSE_LIMIT,
                cancellation: Some(cancellation),
            },
        )
        .map_err(|error| match error {
            HttpExchangeError::Cancelled => ScanError::Cancelled("scan cancelled".into()),
            HttpExchangeError::Failed => ScanError::Unsupported(format!(
                "no response from {}:{}",
                self.endpoint.host, self.endpoint.port
            )),
        })?;
        if status >= 400 {
            return Err(ScanError::Unsupported(format!(
                "eSCL rejected job HTTP {status}"
            )));
        }
        canonical_job_path(&headers, &body, root, &self.endpoint)
            .map(|path| ScanJob { path })
            .ok_or_else(|| ScanError::Unsupported("eSCL did not return job id".into()))
    }

    pub(crate) fn fetch_document(
        &self,
        job: &ScanJob,
        response_limit: u64,
    ) -> std::result::Result<TemporaryOutput, FetchDocumentError> {
        let next = next_document_path(job);
        let deadline = Instant::now() + NEXT_DOCUMENT_DEADLINE;
        let cancellation = self
            .cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .unwrap_or_default();
        loop {
            if self.is_cancelled() {
                return Err(FetchDocumentError::Cancelled);
            }
            let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
                return Err(FetchDocumentError::TimedOut);
            };
            let setup_timeout = timeout.min(NEXT_DOCUMENT_SETUP_TIMEOUT);
            match self.fetch_document_once(
                &next,
                timeout,
                setup_timeout,
                response_limit,
                &cancellation,
            ) {
                Ok(Some(output)) => return Ok(output),
                Ok(None) => {}
                Err(FetchDocumentError::RetryableTimeout) => continue,
                Err(error) => return Err(error),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let pause = NEXT_DOCUMENT_RETRY_DELAY.min(remaining);
            wait_for_document_retry(self, pause)?;
        }
    }

    fn fetch_document_once(
        &self,
        next: &str,
        timeout: Duration,
        setup_timeout: Duration,
        response_limit: u64,
        cancellation: &CancellationToken,
    ) -> std::result::Result<Option<TemporaryOutput>, FetchDocumentError> {
        let (status, output) = http_get_to_temporary_output_with_phase_timeout_and_cancellation(
            &self.endpoint,
            next,
            timeout,
            setup_timeout,
            response_limit,
            Some(cancellation),
        )?;
        if let Some(output) = output {
            return Ok(Some(output));
        }
        if matches!(status, 204 | 404 | 410) {
            return Err(FetchDocumentError::Exhausted);
        }
        if matches!(status, 202 | 409 | 423 | 425 | 429 | 503) {
            return Ok(None);
        }
        Err(FetchDocumentError::Failed(format!(
            "NextDocument failed HTTP {status}"
        )))
    }

    pub(crate) fn cancel_job(&self, job: &ScanJob) {
        let _ = http_exchange_with_cancellation_result(
            &self.endpoint,
            "DELETE",
            &job.path,
            HttpExchangeOptions {
                body: None,
                content_type: None,
                global_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                setup_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                response_header_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                response_limit: JOB_RESPONSE_LIMIT,
                // Cleanup must still be allowed to issue DELETE after the
                // operation token was cancelled. Its independent cap bounds a
                // stalled scanner without extending the cancelled operation.
                cancellation: Some(CancellationToken::new()),
            },
        );
    }

    pub(crate) fn decode_document(
        &self,
        request: &ScanRequest,
        output_file: &TemporaryOutput,
        limits: DocumentLimits,
    ) -> Result<Option<ImageBuffer>> {
        let decode_path = document_path(output_file.path())?;
        let decoded = crate::infrastructure::media::load_image_with_limits(
            &decode_path,
            limits.max_width,
            limits.max_height,
            limits.max_allocation,
        );
        let _ = std::fs::remove_file(&decode_path);
        let Ok(mut image) = decoded else {
            return Ok(None);
        };
        let (target_width, target_height) = requested_document_dimensions(request);
        if target_width > 0
            && target_height > 0
            && (image.width != target_width || image.height != target_height)
        {
            // The scanner can negotiate a different advertised acquisition DPI;
            // preserve ScanRequest's pixel-dimension output contract.
            image = super::super::file::FileDeviceSession::resize_nearest(
                &image,
                target_width,
                target_height,
            )?;
        }
        Ok(Some(image))
    }

    pub(crate) fn acquire_http(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        let mut image = None;
        let summary = self.acquire_http_pages(request, 1, &mut |page| {
            image = Some(page);
            Ok(())
        })?;
        image.ok_or_else(|| {
            let message = match summary.end {
                ScanPagesEnd::FeederExhausted => "eSCL feeder exhausted before returning an image",
                ScanPagesEnd::LimitReached => "eSCL stopped without returning the requested image",
            };
            ScanError::Unsupported(message.into())
        })
    }

    pub(crate) fn acquire_http_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let capabilities = self.scanner_capabilities()?;
        let (settings, representation) = build_settings(request, &capabilities)?;
        // Validate the negotiated source representation before creating a job.
        let document_limits = DocumentLimits::for_request(request, representation)?;
        let job = self.create_job_with_fallback(&capabilities.root, &settings)?;
        self.stream_job_pages(request, max_pages, emit, &job, document_limits)
    }

    fn scanner_capabilities(&self) -> Result<ScannerCapabilities> {
        capabilities_for_endpoint_with_cancellation_result(
            &self.endpoint,
            Instant::now() + ENDPOINT_PROBE_TIMEOUT,
            Some(self.cancellation_token()),
        )
        .map_err(map_capabilities_error)?
        .ok_or_else(|| ScanError::Unsupported("eSCL ScannerCapabilities unavailable".into()))
    }

    fn create_job_with_fallback(&self, root: &str, settings: &[u8]) -> Result<ScanJob> {
        let alternate = if root.eq_ignore_ascii_case("escl") {
            "Scan"
        } else {
            "eSCL"
        };
        let mut last_error = String::new();
        for candidate in [root, alternate] {
            if self.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            match self.create_job(candidate, settings) {
                Ok(job) => return Ok(job),
                Err(error @ ScanError::Cancelled(_)) => return Err(error),
                Err(error) => last_error = error.to_string(),
            }
        }
        Err(ScanError::Unsupported(format!(
            "eSCL acquire failed: {last_error}"
        )))
    }

    pub(crate) fn stream_job_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
        job: &ScanJob,
        document_limits: DocumentLimits,
    ) -> Result<ScanPagesResult> {
        let mut emitted = 0;
        loop {
            self.ensure_stream_active(job, emitted, max_pages)?;
            if emitted == max_pages {
                return Ok(ScanPagesResult::limit_reached(emitted));
            }
            let Some(image) = self.next_stream_image(request, job, document_limits)? else {
                return Ok(ScanPagesResult::feeder_exhausted(emitted));
            };
            if let Err(error) = emit(image) {
                self.cancel_job(job);
                return Err(error);
            }
            emitted += 1;
        }
    }

    fn ensure_stream_active(&self, job: &ScanJob, emitted: u32, maximum: u32) -> Result<()> {
        if self.is_cancelled() {
            self.cancel_job(job);
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if emitted == maximum {
            self.cancel_job(job);
        }
        Ok(())
    }

    fn next_stream_image(
        &self,
        request: &ScanRequest,
        job: &ScanJob,
        limits: DocumentLimits,
    ) -> Result<Option<ImageBuffer>> {
        let output = match self.fetch_document(job, limits.response_limit) {
            Ok(output) => output,
            Err(FetchDocumentError::Exhausted) => return Ok(None),
            Err(error) => return Err(self.stream_fetch_error(job, error)),
        };
        self.decode_document(request, &output, limits)
            .and_then(|image| {
                image.ok_or_else(|| {
                    ScanError::Unsupported("eSCL returned an undecodable raster image".into())
                })
            })
            .map(Some)
            .inspect_err(|_| self.cancel_job(job))
    }

    fn stream_fetch_error(&self, job: &ScanJob, error: FetchDocumentError) -> ScanError {
        self.cancel_job(job);
        match error {
            FetchDocumentError::Cancelled => ScanError::Cancelled("scan cancelled".into()),
            FetchDocumentError::TimedOut | FetchDocumentError::RetryableTimeout => {
                ScanError::Unsupported("eSCL NextDocument timed out".into())
            }
            FetchDocumentError::Failed(error) => {
                ScanError::Unsupported(format!("eSCL acquire failed: {error}"))
            }
            FetchDocumentError::Exhausted => {
                unreachable!("exhaustion is handled before conversion")
            }
        }
    }
}

fn wait_for_document_retry(
    session: &EsclDeviceSession,
    pause: Duration,
) -> std::result::Result<(), FetchDocumentError> {
    let pause_deadline = Instant::now() + pause;
    while Instant::now() < pause_deadline {
        if session.is_cancelled() {
            return Err(FetchDocumentError::Cancelled);
        }
        std::thread::sleep(
            Duration::from_millis(25).min(pause_deadline.saturating_duration_since(Instant::now())),
        );
    }
    Ok(())
}

fn map_capabilities_error(error: HttpExchangeError) -> ScanError {
    match error {
        HttpExchangeError::Cancelled => ScanError::Cancelled("scan cancelled".into()),
        HttpExchangeError::Failed => {
            ScanError::Unsupported("eSCL ScannerCapabilities unavailable".into())
        }
    }
}

pub(crate) fn select_source(
    request: &ScanRequest,
    capabilities: &ScannerCapabilities,
) -> Result<SourceCapabilities> {
    let source = match (request.mode, request.duplex) {
        (ScanMode::Reflective, false) => CapabilitySource::Platen,
        (ScanMode::Reflective, true) => {
            return Err(ScanError::Unsupported(
                "duplex scanning is only supported for document ADF sources".into(),
            ));
        }
        (ScanMode::Document, false) => CapabilitySource::AdfSimplex,
        (ScanMode::Document, true) => CapabilitySource::AdfDuplex,
        (ScanMode::Film, true) => {
            return Err(ScanError::Unsupported(
                "duplex scanning is not supported for film sources".into(),
            ));
        }
        (ScanMode::Film, false) => CapabilitySource::Film,
    };
    if let Some(capabilities) = capabilities.source(source) {
        return Ok(capabilities.clone());
    }
    if source == CapabilitySource::Platen
        && capabilities
            .sources
            .iter()
            .all(|capabilities| capabilities.source.is_none())
    {
        return Ok(capabilities.source_or_default(source));
    }
    let description = match source {
        CapabilitySource::Platen => "platen",
        CapabilitySource::AdfSimplex => "ADF simplex",
        CapabilitySource::AdfDuplex => "ADF duplex",
        CapabilitySource::Film => "film/transparency",
    };
    Err(ScanError::Unsupported(format!(
        "scanner does not advertise a {description} source"
    )))
}

pub(crate) fn select_color_mode(
    request: &ScanRequest,
    source: &SourceCapabilities,
) -> Result<NegotiatedColorMode> {
    let color_mode = compatible_color_mode(request, source)?;
    Ok(NegotiatedColorMode {
        representation: color_representation(&color_mode),
        name: color_mode,
    })
}

fn compatible_color_mode(request: &ScanRequest, source: &SourceCapabilities) -> Result<String> {
    let preferred = preferred_color_modes(request);
    if source.color_modes.is_empty() {
        return Ok(preferred[0].to_string());
    }
    preferred
        .iter()
        .find_map(|wanted| {
            source
                .color_modes
                .iter()
                .find(|advertised| advertised.eq_ignore_ascii_case(wanted))
                .cloned()
        })
        .ok_or_else(|| {
            ScanError::Unsupported(format!(
                "scanner does not advertise a compatible color mode for {:?}",
                request.pixel_format
            ))
        })
}

fn preferred_color_modes(request: &ScanRequest) -> [&'static str; 2] {
    if request.pixel_format == PixelFormat::Gray8 {
        ["Grayscale8", "BlackAndWhite1"]
    } else {
        ["RGB24", "RGB48"]
    }
}

fn color_representation(color_mode: &str) -> DocumentRepresentation {
    match color_mode {
        value if value.eq_ignore_ascii_case("BlackAndWhite1") => {
            DocumentRepresentation::BlackAndWhite1
        }
        value if value.eq_ignore_ascii_case("Grayscale8") => DocumentRepresentation::Grayscale8,
        value if value.eq_ignore_ascii_case("RGB24") => DocumentRepresentation::Rgb24,
        value if value.eq_ignore_ascii_case("RGB48") => DocumentRepresentation::Rgb48,
        _ => unreachable!("selected eSCL color mode is not in the preferred set"),
    }
}

pub(crate) fn select_document_format(source: &SourceCapabilities) -> Result<String> {
    if source.document_formats.is_empty() {
        return Ok("application/octet-stream".into());
    }
    const DECODABLE: [&str; 3] = ["image/png", "image/jpeg", "image/tiff"];
    DECODABLE
        .iter()
        .find_map(|wanted| {
            source
                .document_formats
                .iter()
                .find(|advertised| advertised.eq_ignore_ascii_case(wanted))
                .cloned()
        })
        .ok_or_else(|| {
            ScanError::Unsupported(
                "scanner advertises no decodable raster format (PDF is not a raster image)".into(),
            )
        })
}

pub(crate) fn select_resolution(request: &ScanRequest, source: &SourceCapabilities) -> (u32, u32) {
    let wanted = (request.dpi_x, request.dpi_y);
    source
        .resolutions
        .iter()
        .copied()
        .min_by_key(|(x, y)| x.abs_diff(wanted.0) as u64 + y.abs_diff(wanted.1) as u64)
        .unwrap_or(wanted)
}

pub(crate) fn pixels_to_three_hundredths(pixels: u32, dpi: u32) -> u32 {
    ((pixels as u64)
        .saturating_mul(300)
        .saturating_add((dpi.max(1) / 2) as u64)
        / dpi.max(1) as u64)
        .max(1)
        .min(u32::MAX as u64) as u32
}

pub(crate) fn offset_to_three_hundredths(pixels: u32, dpi: u32) -> u32 {
    if pixels == 0 {
        0
    } else {
        pixels_to_three_hundredths(pixels, dpi)
    }
}

pub(crate) fn region_in_three_hundredths(
    request: &ScanRequest,
    requested_dpi_x: u32,
    requested_dpi_y: u32,
    source: &SourceCapabilities,
) -> (u32, u32, u32, u32) {
    let region = request.region.unwrap_or(crate::domain::image::Rect::new(
        0,
        0,
        request.width.max(1),
        request.height.max(1),
    ));
    let max_width = source.max_width.unwrap_or(u32::MAX).max(1);
    let max_height = source.max_height.unwrap_or(u32::MAX).max(1);
    let x = offset_to_three_hundredths(region.x.max(0) as u32, requested_dpi_x)
        .min(max_width.saturating_sub(1));
    let y = offset_to_three_hundredths(region.y.max(0) as u32, requested_dpi_y)
        .min(max_height.saturating_sub(1));
    let width = pixels_to_three_hundredths(region.width.max(1), requested_dpi_x)
        .min(max_width.saturating_sub(x).max(1));
    let height = pixels_to_three_hundredths(region.height.max(1), requested_dpi_y)
        .min(max_height.saturating_sub(y).max(1));
    (x, y, width, height)
}
