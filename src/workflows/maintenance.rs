//! Device maintenance use cases: open a session, run one hardware action,
//! close it. Kept separate from the acquisition registry so infrastructure
//! stays limited to session lifecycle and dispatch.

use crate::domain::acquisition::ScanRequest;
use crate::domain::image::ImageBuffer;
use crate::domain::processing::{gray_mean, rgb_channel_means};
use crate::error::ScanError;
use crate::infrastructure::acquisition::{
    open_device, AnySession, DeviceMaintenanceCapabilities, DeviceSession,
};

/// Open a device briefly and return its current maintenance capability
/// snapshot. Failure to inspect is represented as unsupported so callers do
/// not dispatch an operation merely because discovery was unavailable.
pub fn maintenance_capabilities(device_id: &str) -> DeviceMaintenanceCapabilities {
    match open_device(device_id) {
        Ok(session) => {
            let capabilities = session.maintenance_capabilities();
            session.close();
            capabilities
        }
        Err(error) => DeviceMaintenanceCapabilities::unsupported(format!(
            "could not inspect maintenance capabilities: {error}"
        )),
    }
}

/// Open session, run calibrate hook, close. Returns session calibrate result.
pub fn calibrate_device(device_id: &str) -> serde_json::Value {
    use serde_json::json;
    run_session_action(device_id, DeviceSession::calibrate, |error| {
        json!({
            "ok": false,
            "status": "error",
            "device_id": device_id,
            "error": error.to_string(),
        })
    })
}

/// Open session, run focus at fractional point, close.
pub fn focus_device(device_id: &str, x: f64, y: f64) -> serde_json::Value {
    use serde_json::json;
    run_session_action(
        device_id,
        |session| session.focus(x, y),
        |error| {
            json!({
                "ok": false,
                "status": "error",
                "device_id": device_id,
                "x_frac": x,
                "y_frac": y,
                "error": error.to_string(),
            })
        },
    )
}

fn run_session_action(
    device_id: &str,
    action: impl FnOnce(&AnySession) -> serde_json::Value,
    on_open_error: impl FnOnce(ScanError) -> serde_json::Value,
) -> serde_json::Value {
    match open_device(device_id) {
        Ok(session) => {
            let mut result = action(&session);
            session.close();
            if let Some(object) = result.as_object_mut() {
                object
                    .entry("device_id")
                    .or_insert_with(|| serde_json::json!(device_id));
            }
            result
        }
        Err(error) => on_open_error(error),
    }
}

/// Compute exposure gains from a preview buffer (mean-channel inverse).
pub fn exposure_gains_from_buffer(image: &ImageBuffer) -> serde_json::Value {
    use serde_json::json;
    if image.bpp() == 1 {
        let mean = gray_mean(image).max(1.0);
        let gain = 128.0 / mean;
        return json!({
            "ok": true,
            "gain_r": gain,
            "gain_g": gain,
            "gain_b": gain,
            "method": "gray-mean",
        });
    }
    let [mr, mg, mb] = rgb_channel_means(image).map(|mean| mean.max(1.0));
    let target = 128.0;
    json!({
        "ok": true,
        "gain_r": target / mr,
        "gain_g": target / mg,
        "gain_b": target / mb,
        "method": "rgb-mean",
    })
}

/// Open session, acquire a small preview, compute exposure gains, close.
pub fn exposure_from_preview(device_id: &str, request: &ScanRequest) -> serde_json::Value {
    use serde_json::json;
    match open_device(device_id) {
        Ok(session) => preview_exposure_result(device_id, request, &session),
        Err(e) => json!({
            "ok": false,
            "status": "error",
            "device_id": device_id,
            "error": e.to_string(),
        }),
    }
}

fn preview_exposure_result(
    device_id: &str,
    request: &ScanRequest,
    session: &AnySession,
) -> serde_json::Value {
    use serde_json::json;
    let mut preview_request = request.clone();
    preview_request.width = preview_request.width.clamp(16, 256);
    preview_request.height = preview_request.height.clamp(16, 256);
    let preview = session.preview(&preview_request);
    session.close();

    match preview {
        Ok(image) => {
            let mut gains = exposure_gains_from_buffer(&image);
            if let Some(object) = gains.as_object_mut() {
                object.insert("device_id".into(), json!(device_id));
                object.insert("status".into(), json!("ok"));
            }
            gains
        }
        Err(error) => json!({
            "ok": false,
            "status": "preview_failed",
            "device_id": device_id,
            "error": error.to_string(),
        }),
    }
}
