# Release notes

Public Rust API changes, newest first. For user-facing behavior, see the
[usage guide](usage.md).

## Unreleased

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
