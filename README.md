# Open Scanline

Open Scanline is a local Rust application for acquiring, processing, and exporting scanned images. It provides a command-line interface and, with the default feature set, a desktop GUI.

The application can run without scanner hardware by using its built-in mock source or a local image file.

| Source | Linux | macOS | Windows | Requirement |
| --- | --- | --- | --- | --- |
| Mock and file | Yes | Yes | Yes | None beyond readable input for file sources |
| SANE | Yes | When SANE is installed | When SANE is installed | A working `scanimage` and a configured device |
| WIA | No | No | Yes | Windows PowerShell, WIA, and a usable scanner |
| eSCL | Yes | Yes | Yes | Reachable AirScan/eSCL scanner over HTTP or HTTPS |
| TWAIN host integration | Host-dependent | Host-dependent | Host-dependent | A separate host or bridge; this is not an acquisition backend |

## Build and test

Install Rust 1.91 or newer, then run:

```bash
cargo build --release
cargo test --all-features
```

The release executable is `target/release/open-scanline` on Linux and macOS, or `target/release/open-scanline.exe` on Windows. To build without the GUI, use `cargo build --no-default-features`.

## Try it

```bash
cargo run -- --version
cargo run -- devices
cargo run -- scan --device mock --out scan.png --width 320 --height 240
cargo run -- process --in scan.png --out processed.png --invert
cargo run -- ocr --in processed.png --offline
cargo run -- batch --device mock --out-dir pages --pages 2 --multipage-out document.pdf
cargo run -- batch --device 'escl:https@scanner.local:443' --allow-unlisted-escl --out-dir sides --source adf --duplex --pages 2
```

`devices` reports the sources visible on the current machine. A reported backend does not guarantee that a particular scanner is installed, connected, or accessible.

## Local demo and GitHub Pages

The mock commands above are the supported hardware-free local demo. To exercise
the desktop surface, run `cargo run -- gui` and select the synthetic mock source
or a local image file; neither path requires a physical scanner.

GitHub Pages is not configured for this repository. The working product is a
native executable whose GUI, scanner adapters, local OCR and model processes,
and network-device access depend on operating-system capabilities that a static
Pages site cannot provide. A Pages site could document Open Scanline, but it
could not run or validate the application, so this repository does not ship a
browser mock that could be mistaken for the product.

## What is included

- Single-page acquisition through mock, file, WIA, SANE, and eSCL sources; one-session ADF batch acquisition through WIA, SANE, and eSCL when the physical device advertises the requested feeder controls
- PNG, JPEG, TIFF, WebP, BMP, GIF, PDF, multipage TIFF, and multipage PDF output paths
- Crop, rotation, flips, color and level adjustments, deskew, white balance, sharpening, document cleanup, and film-oriented processing controls
- A built-in offline template OCR mode, plus optional Tesseract integration
- Searchable and password-protected PDF export, plus validated scanner-profile correction
- Local inference for user-supplied ONNX image models
- A JSON configuration file in the platform-specific `open-scanline` configuration directory

See [usage](docs/usage.md) for command examples, [backend requirements](docs/backends.md) for platform, scanner, and optional-tool details, and [architecture](docs/architecture.md) for the application structure.

The `package --binary <path> --out <zip>` command creates a portable archive, records the packaging host's operating system and architecture in `PORTABLE.txt`, and verifies the archive structure, executable metadata, size, and SHA-256 content hash without launching the supplied program.

## Boundaries

`batch --pages` counts logical image sides. Duplex is available only with the ADF source and requires an even page count; `--pages 2` means the front and back of one sheet. A single-image `scan --duplex` request is rejected instead of silently dropping the rear side.

Mock and file batches are deterministic repeated-page simulations for testing the shared batch pipeline. They do not claim a feeder, source negotiation, or physical duplex behavior.

TWAIN support is host integration only. Open Scanline can run its headless plugin mode for a separate TWAIN-capable host or bridge; it does not ship a native TWAIN Data Source or acquire images directly through TWAIN.

The adapters negotiate the capabilities they can observe, but no compatibility table can guarantee a particular scanner, feeder, driver, firmware, or vendor extension. The automated suite uses simulated adapters and protocol fixtures; validate production hardware on its target operating system before relying on it.

Open Scanline does not include proprietary scanner drivers, firmware, or third-party activation and licensing systems.

## Contributing and security

Read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a change. For vulnerabilities, follow [SECURITY.md](SECURITY.md) rather than opening a public issue.

## License

Project code is [MIT licensed](LICENSE). The embedded searchable-PDF font is covered by the notices and license terms in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
