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
