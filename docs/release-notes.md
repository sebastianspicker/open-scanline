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
