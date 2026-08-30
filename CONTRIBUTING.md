# Contributing

Open Scanline is maintained as a Rust application. Keep changes small, explain their user-visible effect, and include tests when behavior changes.

## Development setup

Install the repository Rust 1.91.0 toolchain. The default build includes the
desktop GUI; the headless profile uses `--no-default-features`.

```bash
sh scripts/check_architecture.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --no-default-features --locked -- -D warnings
cargo test --all-features --locked
cargo test --no-default-features --locked
cargo build --release --all-features --locked
cargo build --release --no-default-features --locked
```

The mock source is available on every platform and is the preferred deterministic path for tests and examples. Do not require physical scanner hardware for ordinary test coverage.

## Backend changes

Keep unavailable scanner services and tools safe to enumerate and fail clearly when used. Preserve the shared scan path for CLI, GUI, batch, and host integration. Changes to a platform adapter should state the platform and external dependency needed to exercise it.

Do not add proprietary scanner drivers, firmware, activation mechanisms, or license material. Do not commit credentials, scanner addresses, captured documents, or other sensitive test data.

## Code placement

Put pure scan, image, and processing behavior in `src/domain/`. Put use-case
coordination and port traits in `src/workflows/`, concrete scanner/media/config
implementations in `src/infrastructure/`, and CLI/GUI/plugin behavior in
`src/inbound/`. Wire native implementations in `src/composition.rs`.

Top-level modules retained for compatibility are public facades, not new
implementation homes. Do not make domain depend on outer layers or let
workflows select infrastructure directly; extend an existing port when a
workflow needs a new capability.

## Before opening a pull request

Describe the commands you ran and their result. If a platform-specific check was not run, say which platform or dependency prevented it. For scanner defects, include the backend and a redacted device description where possible.
