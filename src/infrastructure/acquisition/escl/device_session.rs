use super::*;

impl EsclDeviceSession {
    fn emit_simulated_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        for index in 0..max_pages {
            if self.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            let mut page_request = request.clone();
            page_request.seed = request.seed.saturating_add(index);
            emit(super::super::mock::MockDeviceSession::gradient(
                &page_request,
            )?)?;
        }
        Ok(ScanPagesResult::limit_reached(max_pages))
    }
}

impl DeviceSession for EsclDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        if self.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if self.simulate {
            return super::super::mock::MockDeviceSession::gradient(request);
        }
        self.acquire_http(request)
    }

    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        validate_page_limit(max_pages)?;
        if self.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if self.simulate {
            return self.emit_simulated_pages(request, max_pages, emit);
        }
        self.acquire_http_pages(request, max_pages, emit)
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn bind_cancellation(&self, token: CancellationToken) {
        if self.is_cancelled() {
            token.cancel();
        }
        self.session.bind_cancellation(token);
    }

    fn close(&self) {
        self.session.close();
    }

    fn calibrate(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "backend": "escl",
            "device_id": self.device_id,
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": x.clamp(0.0, 1.0),
            "y_frac": y.clamp(0.0, 1.0),
            "focus": 0.0,
            "backend": "escl",
        })
    }
}
