//! Shared lifecycle state and materialized-output decoding.

use super::seams::{CommandOutput, ImageDecoder};
use crate::domain::acquisition::ScanRequest;
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use std::path::Path;
use std::sync::Mutex;

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
        request: &ScanRequest,
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
            image = crate::infrastructure::acquisition::FileDeviceSession::resize_nearest(
                &image,
                target_width,
                target_height,
            )?;
        }
        Ok(image)
    }
}
