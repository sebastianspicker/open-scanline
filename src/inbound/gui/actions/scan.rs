use super::super::state::{paper_label, GuiState};
use crate::domain::acquisition::{PaperSize, MIN_SCAN_DPI};
use crate::workflows::capture::single::ScanToFileArgs;
use crate::workflows::settings::resolve_scan_extent;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Resolution of physical-device previews.
const PREVIEW_DPI: u32 = 75;

impl GuiState {
    pub(in super::super) fn do_cancel(&mut self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.status = self.translator.t(if self.scanning {
            "status.cancelled"
        } else {
            "status.ready"
        });
    }

    pub(in crate::inbound::gui) fn scan_args(
        &self,
        preview: bool,
    ) -> crate::error::Result<ScanToFileArgs> {
        let out = self.scan_output_path(preview)?;
        let mut pipeline = self.pipeline_prefs();
        let invert_colors = self.invert_colors;
        pipeline.invert = false;
        let cancel_requested = Arc::clone(&self.cancel_requested);
        let (width, height, dpi) = self.scan_geometry(preview);
        // The crop is in full-resolution source pixels; a reduced-resolution
        // preview must select the same physical area.
        if dpi != self.dpi {
            pipeline.crop = pipeline.crop.map(|crop| scale_rect(crop, dpi, self.dpi));
        } else if preview {
            // mock/file previews keep the dpi but shrink the extent.
            let full = self.requested_extent(self.dpi);
            pipeline.crop = pipeline
                .crop
                .and_then(|crop| rescale_crop(crop, full, (width, height)));
        }
        Ok(ScanToFileArgs {
            device: Some(self.device.clone()),
            out,
            width,
            height,
            seed: 1,
            dpi,
            mode: self.scan_mode(),
            duplex: self.duplex,
            pipeline,
            invert_colors,
            use_preview: preview,
            // GUI controls are the authoritative pipeline snapshot for this
            // scan. Passing the persisted config here would re-enable saved
            // effects that the user has since unchecked in the UI.
            config: None,
            on_progress: None,
            cancel_check: Some(Box::new(move || cancel_requested.load(Ordering::SeqCst))),
            raw_out: self.raw_output_path(preview)?,
            overwrite: true,
        })
    }

    fn scan_output_path(&self, preview: bool) -> crate::error::Result<PathBuf> {
        if preview {
            let output_name = crate::domain::settings::validate_output_name(&self.output_name)?;
            Ok(PathBuf::from(&self.output_dir).join(format!("{output_name}_preview.png")))
        } else {
            self.out_path("scan")
        }
    }

    /// Requested pixel extent at `dpi`; 0 on an axis means the device maximum.
    pub(in crate::inbound::gui) fn requested_extent(&self, dpi: u32) -> (u32, u32) {
        let (paper, width, height) = if self.custom_area {
            (None, Some(self.width), Some(self.height))
        } else if self.paper == PaperSize::Auto {
            // Explicit zeros keep stale persisted pixel extents out of play.
            (None, Some(0), Some(0))
        } else {
            (Some(self.paper), None, None)
        };
        resolve_scan_extent(&self.config, paper, width, height, dpi)
    }

    /// Extent and resolution for one acquisition. Physical previews scan the
    /// same area as the final scan at a reduced resolution.
    pub(in crate::inbound::gui) fn scan_geometry(&self, preview: bool) -> (u32, u32, u32) {
        let (width, height) = self.requested_extent(self.dpi);
        if !preview {
            return (width, height, self.dpi);
        }
        if self.device.starts_with("file:") {
            // The file source fits its image inside this box.
            return (320, 240, self.dpi);
        }
        if self.device == "mock" {
            return (width.min(320), height.min(240), self.dpi);
        }
        let dpi = self.dpi.clamp(MIN_SCAN_DPI, PREVIEW_DPI);
        let (width, height) = self.requested_extent(dpi);
        // Custom pixel extents are defined at the full resolution.
        let scale = |pixels: u32| {
            if pixels == 0 || !self.custom_area {
                return pixels;
            }
            scale_pixels(pixels, dpi, self.dpi).max(1)
        };
        (scale(width), scale(height), dpi)
    }

    pub(in crate::inbound::gui) fn scan_area_summary(&self) -> String {
        if self.custom_area {
            return format!(
                "Custom · {} × {} px at {} dpi",
                self.width, self.height, self.dpi
            );
        }
        match self.paper.to_pixels(self.dpi, self.dpi) {
            Some((width, height)) => format!(
                "{} · {} × {} px at {} dpi",
                paper_label(self.paper),
                width,
                height,
                self.dpi
            ),
            None => paper_label(self.paper).into(),
        }
    }

    fn raw_output_path(&self, preview: bool) -> crate::error::Result<Option<PathBuf>> {
        (!preview && self.save_raw)
            .then(|| {
                crate::domain::settings::validate_output_name(&self.output_name).map(
                    |output_name| {
                        PathBuf::from(&self.output_dir)
                            .join(format!("{output_name}_raw_{:03}.tif", self.frame_index))
                    },
                )
            })
            .transpose()
    }
}

/// Convert a pixel length measured at `from_dpi` to `to_dpi`, rounding.
fn scale_pixels(pixels: u32, to_dpi: u32, from_dpi: u32) -> u32 {
    let scaled = (u64::from(pixels) * u64::from(to_dpi) + u64::from(from_dpi) / 2)
        / u64::from(from_dpi.max(1));
    scaled.min(u64::from(u32::MAX)) as u32
}

fn scale_rect(
    rect: crate::domain::image::Rect,
    to_dpi: u32,
    from_dpi: u32,
) -> crate::domain::image::Rect {
    let offset = |value: i32| scale_pixels(value.max(0) as u32, to_dpi, from_dpi) as i32;
    crate::domain::image::Rect::new(
        offset(rect.x),
        offset(rect.y),
        scale_pixels(rect.width, to_dpi, from_dpi).max(1),
        scale_pixels(rect.height, to_dpi, from_dpi).max(1),
    )
}

/// Map a crop from the `full` scan extent onto a smaller `preview` extent. With
/// an unknown (zero) full axis the preview drops the crop instead of failing.
fn rescale_crop(
    rect: crate::domain::image::Rect,
    full: (u32, u32),
    preview: (u32, u32),
) -> Option<crate::domain::image::Rect> {
    if full == preview {
        return Some(rect);
    }
    if full.0 == 0 || full.1 == 0 {
        return None;
    }
    let x = |value: i32| scale_pixels(value.max(0) as u32, preview.0, full.0) as i32;
    let y = |value: i32| scale_pixels(value.max(0) as u32, preview.1, full.1) as i32;
    Some(crate::domain::image::Rect::new(
        x(rect.x),
        y(rect.y),
        scale_pixels(rect.width, preview.0, full.0).max(1),
        scale_pixels(rect.height, preview.1, full.1).max(1),
    ))
}

#[cfg(test)]
mod tests {
    use super::rescale_crop;
    use crate::domain::image::Rect;

    #[test]
    fn preview_crop_follows_the_extent_ratio() {
        let crop = Rect::new(100, 100, 800, 600);
        assert_eq!(rescale_crop(crop, (1280, 960), (1280, 960)), Some(crop));
        assert_eq!(
            rescale_crop(crop, (1280, 960), (320, 240)),
            Some(Rect::new(25, 25, 200, 150))
        );
        assert_eq!(rescale_crop(crop, (0, 0), (320, 240)), None);
    }
}
