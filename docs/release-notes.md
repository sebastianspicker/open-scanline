# Release notes

Public Rust API changes, newest first. For user-facing behavior, see the
[usage guide](usage.md).

## 0.1.0 (alpha)

The package version drops from `1.0.0` to `0.1.0`. `VERSION` is now taken from
`Cargo.toml`, so the two cannot drift apart.

### Public API

- `ScanToFileArgs`, `BatchScanArgs` and `ProcessOptions` gain a public
  `overwrite: bool` field. Code that builds them with a struct literal must set
  it (`ScanToFileArgs` and `BatchScanArgs` also accept `..Default::default()`;
  `ProcessOptions` has no `Default`). With `false`, which is the default, the
  workflows refuse to replace an existing output file and return
  `ScanError::Invalid("output exists: …")`. Set it to `true` to keep the old
  behaviour of replacing files.
- `ScanRequest`, `ScanToFileArgs` and `BatchScanArgs` now default to a width
  and height of `0`, where they used to default to 320 × 240. A zero axis means
  "the device's maximum scan area", and physical backends no longer resize
  their output to the requested pixel size. Mock sources still produce
  320 × 240 for a zero axis.
- `AppConfig` gains `paper_size` (`"auto"` by default) and `config_version`.
  `default_width` and `default_height` now default to `0`. Loading a config
  file without `config_version` whose extents are exactly the old 320 × 240
  default resets them to `0`.

### Public API (repo audit fixes)

Changed behaviour:

- `imaging::load_image` applies the EXIF orientation, so a photo tagged
  "rotate 90" loads upright. `imaging::load_image_with_limits` still returns the
  stored pixel layout.
- `pipeline::auto_deskew` now rotates against the estimated skew; it used to
  double it. `pipeline::deskew` is unchanged: a positive angle turns the page
  clockwise on screen.
- `DeviceInfo::new` drops control, bidi and zero-width characters from the name
  and keeps at most 128 characters.
- `colorize` returns `ScanError::Invalid` for an unknown mode, and
  `grain_reduction` and `infrared_clean` do the same for an unknown tier,
  instead of falling back to sepia or "medium".
- `escl::parse_job_id` and eSCL job handling accept only job paths of the form
  `/<root>/ScanJobs/<id>`.

### Public API (code-quality fixes)

New:

- `scan::run_scan_to_file_with_options(args, CaptureOptions)`,
  `batch::run_batch_scan_with_options(args, BatchCaptureOptions)` and
  `process::process_image_file_with_options(&options, ProcessRunOptions)` take
  the export, cancellation and device-policy controls as an options struct. The
  structs are public, with public fields and `Default`.

Removed:

- The twelve permutation wrappers they replace:
  - `scan::run_scan_to_file_with_token`, `_with_export_options`,
    `_with_export_options_and_token` and
    `_with_export_options_and_token_and_policy`;
  - `batch::run_batch_scan_with_token`, `_with_export_options`,
    `_with_export_options_and_token`,
    `_with_export_options_and_token_and_policy` and
    `_with_export_options_and_cancel`;
  - `process::process_image_file_with_token`, `_with_export_options` and
    `_with_export_options_and_token`.
  - `run_scan_to_file`, `run_batch_scan` and `process_image_file` are unchanged.
- `ml::run_user_onnx` and `ml::run_user_onnx_with_options`, which always failed.
  Use `OnnxRuntime::from_worker` or `run_user_onnx_with_worker`. The
  `ml_module_info` JSON drops its `run_user_onnx` key.
- `imaging::append_page_to_multipage`. Its PDF branch replaced an existing
  document with a single page.
- `icc::build_icc_bytes` and `icc::apply_scanner_profile`, which had no callers.
- `sane::scanimage_command` and `FileDeviceSession::resize_nearest`.

Changed:

- `ExportOptions` and `PdfOptions` implement `Drop`, which zeroes the PDF
  password, and `ExportOptions` debug output redacts it. Build them with
  `let mut o = ExportOptions::default(); o.pdf_password = …;` instead of the
  `..Default::default()` struct-update syntax.
- `OnnxOutputSummary.sample` is `Vec<Option<f64>>`; non-finite values are `None`.
- `resolve_device_id(None)` returns `mock` without running device discovery.
  Only the exact id `mock` opens the synthetic scanner.
- `escl::open_explicit_id("escl:sim")` needs `OPEN_SCANLINE_SIMULATE_BACKENDS=1`.
- `deskew` and `adjust_levels` return `ScanError::Invalid` for a non-finite angle
  or gamma.
- `invert`, `adjust_levels`, `apply_curves` and `adjust_brightness_contrast`
  keep the alpha byte of `Rgba8` pixels.
- `AppConfig` loading rejects a `rotate` that is not a multiple of 90, a
  non-positive or non-finite `levels_gamma`, a `deskew_angle` outside ±45, and
  an unknown `film_type`.
- The plugin status JSON reports `ok: false` with an `error` field, and exit 1,
  when the configuration cannot be loaded.

## Earlier changes

### Public API

- The public `OcrEngine` enum adds the `Ocrs` variant. This is additive for
  callers that do not exhaustively match the enum, but downstream exhaustive
  matches must handle the new variant. Existing `Offline` and `Tesseract`
  behavior and the default template-OCR configuration are unchanged.
- The public `ml` facade adds `OnnxRuntime` for explicitly selected worker
  execution. The historical no-worker ONNX helpers remain present, but are
  deprecated and fail closed; they no longer discover sibling executables.
- `CancellationToken` adds `check(context)`, which returns
  `ScanError::Cancelled(context)` once the token is cancelled.
- `OcrEngine` adds `NAMES`, `as_str`, and a case-insensitive `FromStr` whose
  error is `ScanError::Invalid`.
- `imaging::image_buffer_to_rgba` validates the packed buffer first and returns
  an error for an inconsistent image instead of panicking or overflowing.
- `device::exposure_gains_from_buffer` counts pixels without `u32` overflow.
- Internal layering changed (no ports or composition root; see the
  [architecture guide](architecture.md)). Every public path and signature is
  otherwise unchanged.
