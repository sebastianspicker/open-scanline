# Contributing

Open Scanline is maintained as a Rust application. Keep changes small, explain
their user-visible effect, and include tests when behavior changes.

## Development setup

Install the repository Rust 1.91.0 toolchain. The default build includes the
desktop GUI, OCRS, and ONNX. Use `--no-default-features` for the core-only
profile, or `--no-default-features --features ocrs,onnx` for full headless
support.

Run the full gate before opening a pull request:

```bash
sh scripts/check_architecture.sh
bash scripts/check_code_quality.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --no-default-features --locked -- -D warnings
cargo test --all-features --locked
cargo test --no-default-features --locked
cargo build --release --all-features --locked
cargo build --release --no-default-features --locked
cargo clippy --all-targets --no-default-features --features ocrs,onnx --locked -- -D warnings
cargo test --no-default-features --features ocrs,onnx --locked
cargo build --release --no-default-features --features ocrs,onnx --locked
cargo check --all-targets --no-default-features --features gui --locked
cargo check --all-targets --no-default-features --features ocrs --locked
cargo check --all-targets --no-default-features --features onnx --locked
```

The handwritten-code gate pins Ruff 0.15.20, Lizard 1.23.0, and JSCPD 5.1.2. It
limits functions to CCN 8, 50 NLOC, and eight parameters, limits files to 500
Lizard NLOC, keeps Rust sources below 600 physical lines, and allows at most 0.5%
duplication.

The mock source is available on every platform and is the preferred deterministic
path for tests and examples. Do not require physical scanner hardware for ordinary
test coverage.

## Backend changes

Keep unavailable scanner services and tools safe to enumerate, and make them fail
clearly when used. Preserve the shared scan path for CLI, GUI, batch, and host
integration. When you change a platform adapter, state the platform and external
dependency needed to exercise it.

Do not add proprietary scanner drivers, firmware, activation mechanisms, or
license material. Do not commit credentials, scanner addresses, captured
documents, or other sensitive test data.

## Code placement

Put pure scan, image, and processing behavior in `src/domain/`. Put use-case
coordination and port traits in `src/workflows/`, concrete scanner/media/config
implementations in `src/infrastructure/`, and CLI/GUI/plugin behavior in
`src/inbound/`. Wire native implementations in `src/composition.rs`.

Top-level modules retained for compatibility are public facades, not new
implementation homes. Do not make domain depend on outer layers or let workflows
select infrastructure directly; extend an existing port when a workflow needs a
new capability.

## Documentation

Repository documentation lives in `docs/`. The README is the user entry point,
`docs/usage.md` owns detailed commands, and `docs/backends.md` owns platform and
scanner constraints. Keep the architecture guide consistent with
`scripts/check_architecture.sh` and `tests/public_api_contract.rs`.

`docs/index.html` is the GitHub Pages landing page, served together with
`docs/assets/` from the `/docs` folder. `docs/.nojekyll` keeps GitHub Pages from
running Jekyll over the documentation sources, so the Markdown files are read on
GitHub rather than rendered as a site. Screenshots for the page and the README
live in `docs/assets/screenshots/`; keep them free of local paths and personal
content.

## Before opening a pull request

Describe the commands you ran and their result. If you could not run a
platform-specific check, say which platform or dependency prevented it. For
scanner defects, include the backend and a redacted device description where
possible.

Performance workloads and measurement limits are documented in
[performance checks](docs/performance.md). Timing thresholds do not belong in the
ordinary CI gate.
