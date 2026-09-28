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
        let color =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
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

fn preview_fingerprint(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}
