use super::common::{parse_crop, print_wrote};
use crate::cli::args::ScanSource;
use crate::config::AppConfig;
use crate::core::{PipelinePrefs, Rect, Rotate};
use crate::device::{CancellationToken, DeviceOpenPolicy};
use crate::scan::{run_scan_to_file_with_export_options_and_token_and_policy, ScanToFileArgs};
use std::path::PathBuf;

pub(super) struct Request {
    pub(super) config: AppConfig,
    pub(super) export: crate::ExportOptions,
    pub(super) acquisition: Acquisition,
    pub(super) overrides: PipelineOverrides,
    pub(super) auto_crop: Option<bool>,
    pub(super) auto_orient: Option<bool>,
}

pub(super) struct Acquisition {
    pub(super) device: Option<String>,
    pub(super) allow_unlisted_escl: bool,
    pub(super) out: PathBuf,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) seed: u32,
    pub(super) dpi: Option<u32>,
    pub(super) source: Option<ScanSource>,
    pub(super) duplex: Option<bool>,
    pub(super) invert: Option<bool>,
    pub(super) raw_out: Option<PathBuf>,
}

#[derive(Default)]
pub(super) struct PipelineOverrides {
    pub(super) geometry: GeometryOverrides,
    pub(super) color: ColorOverrides,
    pub(super) filters: FilterOverrides,
}

#[derive(Default)]
pub(super) struct GeometryOverrides {
    pub(super) rotate: Option<i32>,
    pub(super) flip_h: Option<bool>,
    pub(super) flip_v: Option<bool>,
    pub(super) crop: Option<String>,
    pub(super) auto_deskew: Option<bool>,
    pub(super) deskew: Option<f64>,
}

#[derive(Default)]
pub(super) struct ColorOverrides {
    pub(super) brightness: Option<i32>,
    pub(super) contrast: Option<i32>,
    pub(super) saturation: Option<f64>,
    pub(super) hue: Option<f64>,
    pub(super) curves: Option<Vec<[i32; 2]>>,
    pub(super) desaturate: Option<bool>,
    pub(super) levels_black: Option<i32>,
    pub(super) levels_white: Option<i32>,
    pub(super) levels_gamma: Option<f64>,
    pub(super) white_balance: Option<bool>,
    pub(super) auto_levels: Option<bool>,
}

#[derive(Default)]
pub(super) struct FilterOverrides {
    pub(super) infrared_clean: Option<String>,
    pub(super) descreen: Option<bool>,
    pub(super) descreen_dpi: Option<i32>,
    pub(super) sharpen: Option<f64>,
    pub(super) film_type: Option<String>,
    pub(super) restore_colors: Option<bool>,
    pub(super) restore_fading: Option<bool>,
    pub(super) grain_reduction: Option<String>,
    pub(super) flatten: Option<bool>,
    pub(super) hole_punch: Option<bool>,
    pub(super) colorize_mode: Option<String>,
}

pub(super) fn run(mut request: Request, cancellation: CancellationToken) -> i32 {
    let mut pipeline = pipeline_with_overrides(&request.config, &request.overrides);
    apply_explicit_bool(&mut pipeline.auto_crop, request.auto_crop);
    apply_explicit_bool(&mut pipeline.auto_orient, request.auto_orient);

    let allow_unlisted_escl = request.acquisition.allow_unlisted_escl;
    let args = scan_args(request.acquisition, request.config, pipeline);
    let result = run_scan_to_file_with_export_options_and_token_and_policy(
        args,
        &request.export,
        cancellation,
        DeviceOpenPolicy {
            allow_unlisted_escl,
        },
    );
    super::router::clear_pdf_password(&mut request.export.pdf_password);
    match result {
        Ok(path) => finish_scan(path),
        Err(error) => report_scan_error(error),
    }
}

fn scan_args(
    acquisition: Acquisition,
    config: AppConfig,
    pipeline: PipelinePrefs,
) -> ScanToFileArgs {
    ScanToFileArgs {
        device: Some(
            acquisition
                .device
                .clone()
                .unwrap_or_else(|| config.last_device_id.clone()),
        ),
        out: acquisition.out.clone(),
        width: acquisition.width.unwrap_or(config.default_width),
        height: acquisition.height.unwrap_or(config.default_height),
        seed: acquisition.seed,
        dpi: acquisition.dpi.unwrap_or(config.default_dpi),
        mode: acquisition
            .source
            .map(ScanSource::mode)
            .unwrap_or(config.scan_mode),
        duplex: acquisition.duplex.unwrap_or(config.duplex),
        pipeline,
        invert_colors: acquisition.invert.unwrap_or(config.invert_colors),
        use_preview: false,
        // The pipeline and acquisition values above already resolve config and
        // explicit CLI values. Passing config again would re-enable a value the
        // CLI explicitly disabled.
        config: None,
        on_progress: None,
        cancel_check: None,
        raw_out: acquisition.raw_out,
    }
}

pub(super) fn pipeline_with_overrides(
    config: &AppConfig,
    overrides: &PipelineOverrides,
) -> PipelinePrefs {
    let mut pipeline = config.to_pipeline_prefs();
    apply_geometry_overrides(&mut pipeline, &overrides.geometry);
    apply_color_overrides(&mut pipeline, &overrides.color);
    apply_filter_overrides(&mut pipeline, &overrides.filters);
    pipeline
}

fn apply_geometry_overrides(pipeline: &mut PipelinePrefs, overrides: &GeometryOverrides) {
    if let Some(rotate) = overrides.rotate {
        pipeline.rotate = Rotate::from_degrees(rotate);
    }
    apply_explicit_bool(&mut pipeline.flip_h, overrides.flip_h);
    apply_explicit_bool(&mut pipeline.flip_v, overrides.flip_v);
    apply_crop(pipeline, overrides.crop.as_deref());
    apply_explicit_bool(&mut pipeline.auto_deskew, overrides.auto_deskew);
    if let Some(deskew) = overrides.deskew {
        pipeline.deskew_angle = deskew;
    }
}

fn apply_crop(pipeline: &mut PipelinePrefs, value: Option<&str>) {
    let Some(crop) = parse_crop(value) else {
        return;
    };
    if crop[2] > 0 && crop[3] > 0 {
        pipeline.crop = Some(Rect::new(crop[0], crop[1], crop[2] as u32, crop[3] as u32));
    }
}

fn apply_color_overrides(pipeline: &mut PipelinePrefs, overrides: &ColorOverrides) {
    if let Some(brightness) = overrides.brightness {
        pipeline.brightness = brightness;
    }
    if let Some(contrast) = overrides.contrast {
        pipeline.contrast = contrast;
    }
    if let Some(saturation) = overrides.saturation {
        pipeline.saturation = saturation;
    }
    if let Some(hue) = overrides.hue {
        pipeline.hue = hue;
    }
    if let Some(curves) = &overrides.curves {
        pipeline.curves = Some(curves.clone());
    }
    apply_explicit_bool(&mut pipeline.desaturate, overrides.desaturate);
    if let Some(levels_black) = overrides.levels_black {
        pipeline.levels_black = levels_black;
    }
    if let Some(levels_white) = overrides.levels_white {
        pipeline.levels_white = levels_white;
    }
    if let Some(levels_gamma) = overrides.levels_gamma {
        pipeline.levels_gamma = levels_gamma;
    }
    apply_explicit_bool(&mut pipeline.white_balance, overrides.white_balance);
    apply_explicit_bool(&mut pipeline.auto_levels, overrides.auto_levels);
}

fn apply_filter_overrides(pipeline: &mut PipelinePrefs, overrides: &FilterOverrides) {
    apply_enabled_tier(
        &mut pipeline.infrared_clean,
        overrides.infrared_clean.as_deref(),
    );
    apply_explicit_bool(&mut pipeline.descreen, overrides.descreen);
    if let Some(descreen_dpi) = overrides.descreen_dpi {
        pipeline.descreen_dpi = descreen_dpi;
    }
    if let Some(sharpen) = overrides.sharpen {
        pipeline.sharpen_amount = sharpen;
    }
    apply_optional_value(&mut pipeline.film_type, overrides.film_type.as_deref());
    apply_explicit_bool(&mut pipeline.restore_colors, overrides.restore_colors);
    apply_explicit_bool(&mut pipeline.restore_fading, overrides.restore_fading);
    apply_enabled_tier(
        &mut pipeline.grain_reduction,
        overrides.grain_reduction.as_deref(),
    );
    apply_explicit_bool(&mut pipeline.flatten, overrides.flatten);
    apply_explicit_bool(&mut pipeline.hole_punch, overrides.hole_punch);
    apply_optional_value(
        &mut pipeline.colorize_mode,
        overrides.colorize_mode.as_deref(),
    );
}

fn apply_enabled_tier(target: &mut Option<String>, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "off" | "none" | "false"
    ) {
        *target = None;
    } else {
        *target = Some(value.to_string());
    }
}

fn apply_optional_value(target: &mut Option<String>, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if matches!(value.to_ascii_lowercase().as_str(), "off" | "none") {
        *target = None;
    } else if !value.is_empty() {
        *target = Some(value.to_string());
    }
}

pub(super) fn apply_explicit_bool(target: &mut bool, value: Option<bool>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn finish_scan(path: PathBuf) -> i32 {
    print_wrote(&path);
    0
}

fn report_scan_error(error: crate::core::ScanError) -> i32 {
    eprintln!("scan error: {error}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_false_and_off_values_override_configured_pipeline_preferences() {
        let config = AppConfig {
            descreen: true,
            flatten: true,
            infrared_clean: Some("medium".into()),
            grain_reduction: Some("light".into()),
            film_type: Some("negative".into()),
            colorize_mode: Some("auto".into()),
            ..AppConfig::default()
        };
        let overrides = PipelineOverrides {
            filters: FilterOverrides {
                infrared_clean: Some("off".into()),
                descreen: Some(false),
                film_type: Some("none".into()),
                grain_reduction: Some("off".into()),
                flatten: Some(false),
                colorize_mode: Some("off".into()),
                ..FilterOverrides::default()
            },
            ..PipelineOverrides::default()
        };

        let pipeline = pipeline_with_overrides(&config, &overrides);

        assert!(!pipeline.descreen);
        assert!(!pipeline.flatten);
        assert!(pipeline.infrared_clean.is_none());
        assert!(pipeline.grain_reduction.is_none());
        assert!(pipeline.film_type.is_none());
        assert!(pipeline.colorize_mode.is_none());
    }
}
