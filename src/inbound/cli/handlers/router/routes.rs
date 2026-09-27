use super::export;
use crate::inbound::cli::args::{
    Adjustments, BatchOptions, ExportOptions, FilterOptions, ProcessOptions, ScanOptions,
};
use crate::inbound::cli::handlers::{cancellation, commands, process, scan};
use crate::infrastructure::config::json::load_config;
use crate::workflows::settings::AppConfig;
use std::path::Path;

pub(super) fn scan(options: ScanOptions, config_path: Option<&Path>) -> i32 {
    let mut options = options;
    let export_options = std::mem::take(&mut options.export);
    let (config, export) = match configured_export(config_path, export_options) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let request = scan_request(options, config, export);
    cancellation::with_registered_token(move |cancellation| scan::run(request, cancellation))
}

fn scan_request(
    options: ScanOptions,
    config: AppConfig,
    export: crate::ExportOptions,
) -> scan::Request {
    scan::Request {
        config,
        export,
        acquisition: scan::Acquisition {
            device: options.acquisition.device,
            allow_unlisted_escl: options.acquisition.allow_unlisted_escl,
            out: options.out,
            width: options.acquisition.width,
            height: options.acquisition.height,
            seed: options.acquisition.seed,
            dpi: options.acquisition.dpi,
            source: options.acquisition.source,
            duplex: options.duplex,
            invert: options.invert,
            raw_out: options.raw_out,
        },
        overrides: pipeline_overrides(
            options.adjustments,
            options.filters,
            options.auto_deskew,
            options.deskew,
            options.white_balance,
            options.auto_levels,
        ),
        auto_crop: options.auto_crop,
        auto_orient: options.auto_orient,
    }
}

pub(super) fn process(options: ProcessOptions, config_path: Option<&Path>) -> i32 {
    let ProcessOptions {
        inp,
        out,
        export: export_options,
        adjustments,
        invert,
        auto_levels,
        auto_crop,
        auto_orient,
        auto_deskew,
        deskew,
        white_balance,
        filters,
        quality,
    } = options;
    let (config, export) = match configured_export(config_path, export_options) {
        Ok(value) => value,
        Err(code) => return code,
    };
    cancellation::with_registered_token(move |cancellation| {
        process::run(
            process::Request {
                config,
                inp,
                out,
                export,
                overrides: pipeline_overrides(
                    adjustments,
                    filters,
                    auto_deskew,
                    deskew,
                    white_balance,
                    auto_levels,
                ),
                invert,
                auto_crop,
                auto_orient,
                quality,
            },
            cancellation,
        )
    })
}

pub(super) fn batch(options: BatchOptions, config_path: Option<&Path>) -> i32 {
    let mut options = options;
    let (config, export) = match configured_export(config_path, std::mem::take(&mut options.export))
    {
        Ok(value) => value,
        Err(code) => return code,
    };
    let request = batch_request(options, config, export);
    cancellation::with_registered_token(move |cancellation| commands::batch(request, cancellation))
}

fn batch_request(
    options: BatchOptions,
    config: AppConfig,
    export: crate::ExportOptions,
) -> commands::BatchRequest {
    commands::BatchRequest {
        config,
        device: options.acquisition.device,
        allow_unlisted_escl: options.acquisition.allow_unlisted_escl,
        out_dir: options.out_dir,
        pages: options.pages,
        width: options.acquisition.width,
        height: options.acquisition.height,
        seed: options.acquisition.seed,
        dpi: options.acquisition.dpi,
        source: options.acquisition.source,
        duplex: options.duplex,
        format: options.format,
        overrides: pipeline_overrides(
            options.adjustments,
            options.filters,
            options.auto_deskew,
            options.deskew,
            options.white_balance,
            options.auto_levels,
        ),
        auto_crop: options.auto_crop,
        auto_orient: options.auto_orient,
        invert: options.invert,
        multipage_tiff: options.multipage_tiff,
        multipage_pdf: options.multipage_pdf,
        multipage_out: options.multipage_out,
        export,
        contact_sheet: options.contact_sheet,
    }
}

fn configured_export(
    config_path: Option<&Path>,
    overrides: ExportOptions,
) -> Result<(AppConfig, crate::ExportOptions), i32> {
    let config = load_config(config_path).map_err(|error| {
        eprintln!("config error: {error}");
        1
    })?;
    let export = export::build(&config, overrides).map_err(export::report_error)?;
    Ok((config, export))
}

fn pipeline_overrides(
    adjustments: Adjustments,
    filters: FilterOptions,
    auto_deskew: Option<bool>,
    deskew: Option<f64>,
    white_balance: Option<bool>,
    auto_levels: Option<bool>,
) -> scan::PipelineOverrides {
    let (geometry, color) =
        adjustment_overrides(adjustments, auto_deskew, deskew, white_balance, auto_levels);
    scan::PipelineOverrides {
        geometry,
        color,
        filters: filter_overrides(filters),
    }
}

fn adjustment_overrides(
    adjustments: Adjustments,
    auto_deskew: Option<bool>,
    deskew: Option<f64>,
    white_balance: Option<bool>,
    auto_levels: Option<bool>,
) -> (scan::GeometryOverrides, scan::ColorOverrides) {
    let Adjustments {
        rotate,
        flip_h,
        flip_v,
        crop,
        brightness,
        contrast,
        saturation,
        hue,
        curves,
        desaturate,
        levels_black,
        levels_white,
        levels_gamma,
    } = adjustments;
    (
        scan::GeometryOverrides {
            rotate,
            flip_h,
            flip_v,
            crop,
            auto_deskew,
            deskew,
        },
        scan::ColorOverrides {
            brightness,
            contrast,
            saturation,
            hue,
            curves: curves.map(|curves| curves.0),
            desaturate,
            levels_black,
            levels_white,
            levels_gamma,
            white_balance,
            auto_levels,
        },
    )
}

fn filter_overrides(filters: FilterOptions) -> scan::FilterOverrides {
    scan::FilterOverrides {
        infrared_clean: filters.infrared_clean,
        descreen: filters.descreen,
        descreen_dpi: filters.descreen_dpi,
        sharpen: filters.sharpen,
        film_type: filters.film_type,
        restore_colors: filters.restore_colors,
        restore_fading: filters.restore_fading,
        grain_reduction: filters.grain_reduction,
        flatten: filters.flatten,
        hole_punch: filters.hole_punch,
        colorize_mode: filters.colorize_mode,
    }
}
