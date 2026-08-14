# Contributing

Open Scanline is maintained as a Rust application. Keep changes small, explain their user-visible effect, and include tests when behavior changes.

## Development setup

Install a current stable Rust toolchain. The default build includes the desktop GUI; the core can also be built without it.

```bash
cargo fmt --check
cargo test --all-features
cargo test --no-default-features
cargo build --release
```

The mock source is available on every platform and is the preferred deterministic path for tests and examples. Do not require physical scanner hardware for ordinary test coverage.

## Backend changes

Keep unavailable scanner services and tools safe to enumerate and fail clearly when used. Preserve the shared scan path for CLI, GUI, batch, and host integration. Changes to a platform adapter should state the platform and external dependency needed to exercise it.

Do not add proprietary scanner drivers, firmware, activation mechanisms, or license material. Do not commit credentials, scanner addresses, captured documents, or other sensitive test data.

## Before opening a pull request

Describe the commands you ran and their result. If a platform-specific check was not run, say which platform or dependency prevented it. For scanner defects, include the backend and a redacted device description where possible.
