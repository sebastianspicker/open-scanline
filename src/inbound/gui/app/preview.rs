use super::*;
use crate::domain::processing::histogram;
use crate::inbound::gui::state::PreviewStatistics;

impl OpenScanlineApp {
    /// Texture, statistics and failure entries share one path/revision fingerprint.
    pub(in crate::inbound::gui) fn ensure_preview_texture(&mut self, ctx: &egui::Context) {
        self.ensure_preview_with(ctx, |path| load_image(path));
    }

    fn ensure_preview_with(
        &mut self,
        ctx: &egui::Context,
        decode: impl FnOnce(&Path) -> crate::error::Result<crate::domain::image::ImageBuffer>,
    ) {
        let path = self.state.last_image.clone();
        let fingerprint = path.as_deref().and_then(preview_fingerprint);
        if self.preview_tex_path == path
            && self.preview_tex_fp == fingerprint
            && self.preview_revision == self.state.preview_revision
        {
            return;
        }
        self.preview_tex = None;
        self.state.preview = None;
        self.state.hist_summary.clear();
        self.preview_tex_path = path.clone();
        self.preview_tex_fp = fingerprint;
        self.preview_revision = self.state.preview_revision;
        let Some(path) = path else {
            return;
        };
        let result = decode(&path).and_then(|image| self.cache_preview(ctx, &image));
        if let Err(error) = result {
            self.state.set_error(error);
        }
    }

    fn cache_preview(
        &mut self,
        ctx: &egui::Context,
        image: &crate::domain::image::ImageBuffer,
    ) -> crate::error::Result<()> {
        let (width, height, rgba) = image_buffer_to_rgba(image)?;
        let histogram = histogram(image)?;
        let luma = histogram["luma"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_u64().unwrap_or(0))
                    .collect()
            })
            .unwrap_or_default();
        // GPUs reject textures above GL_MAX_TEXTURE_SIZE (egui panics), so shrink big scans.
        let max_side = ctx.input(|input| input.max_texture_side).min(4096) as u32;
        let (shown_w, shown_h) = fit_preview(width, height, max_side);
        let rgba = if (shown_w, shown_h) == (width, height) {
            rgba
        } else {
            let full = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
                crate::error::ScanError::Image("preview buffer size mismatch".into())
            })?;
            image::imageops::thumbnail(&full, shown_w, shown_h).into_raw()
        };
        let color =
            egui::ColorImage::from_rgba_unmultiplied([shown_w as usize, shown_h as usize], &rgba);
        self.preview_tex = Some(ctx.load_texture("preview", color, egui::TextureOptions::LINEAR));
        self.state.preview = Some(PreviewStatistics {
            width,
            height,
            format: image.pixel_format,
            luma,
        });
        self.state.hist_summary = format!(
            "histogram count={}",
            histogram["count"].as_u64().unwrap_or(0)
        );
        Ok(())
    }
}

/// Scale `width` x `height` down, keeping the aspect ratio, so neither side exceeds `max`.
fn fit_preview(width: u32, height: u32, max: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max {
        return (width, height);
    }
    let scale = |side: u32| ((u64::from(side) * u64::from(max) / u64::from(longest)) as u32).max(1);
    (scale(width), scale(height))
}

fn preview_fingerprint(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::fit_preview;

    #[test]
    fn fit_preview_keeps_aspect_and_limit() {
        assert_eq!(fit_preview(20000, 3000, 4096), (4096, 614));
        assert_eq!(fit_preview(3000, 20000, 4096), (614, 4096));
        assert_eq!(fit_preview(4096, 100, 4096), (4096, 100));
        assert_eq!(fit_preview(1, 1, 4096), (1, 1));
        assert_eq!(fit_preview(20000, 1, 4096), (4096, 1));
    }
}
