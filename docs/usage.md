# Using Open Scanline

Run `open-scanline --help` to see the available commands. The examples below use
`cargo run --` while developing; replace that prefix with the installed
executable when appropriate. The default feature profile includes the GUI; use
`--no-default-features` for a headless build or run.

## Acquire and process an image

The mock source produces deterministic image data and does not need hardware:

```bash
cargo run -- scan --device mock --out scan.png --width 320 --height 240 --dpi 150
cargo run -- process --in scan.png --out processed.png --rotate 90 --auto-levels
```

To use a local image as a source, pass a `file:` device identifier:

```bash
cargo run -- scan --device file:/absolute/path/to/source.png --out copy.png
```

Use `devices` to list discovered devices and backend availability before selecting WIA, SANE, or eSCL hardware:

```bash
cargo run -- devices
```

`--source` selects `flatbed`, `adf`, or `film`. A crop is forwarded to a hardware backend as an acquisition region when that backend supports regions. A single `scan` always produces one image, so `scan --duplex` fails with guidance to use `batch`; it never discards the back side silently.

Color and processing controls are available on `scan`, `batch`, and `process`:

```bash
cargo run -- process --in scan.png --out corrected.png \
  --saturation 18 --hue -6 --curves 0:0,96:112,192:210,255:255
```

Curve coordinates must be in 0 through 255 and x coordinates must increase strictly.

`scan`, `batch`, and `process` start with the selected JSON configuration. Supplied value options replace configured values. Configuration-aware switches accept the existing bare form to enable a step, such as `--auto-levels`, and `=false` to disable a configured step, such as `--auto-levels=false` or `--duplex=false`; omitting the switch retains the configured value. `off` or `none` clears configured infrared-clean and grain-reduction tiers, film type, and colorization mode, for example `--infrared-clean=off` or `--film-type none`.

## Batch output

The batch command writes individual pages and can create a multipage TIFF or PDF:

```bash
cargo run -- batch --device mock --out-dir pages --pages 3 --multipage-pdf document.pdf
cargo run -- batch --device mock --out-dir pages --pages 3 --multipage-out document.tif
cargo run -- batch --device 'escl:https@scanner.local:443' --out-dir sides \
  --allow-unlisted-escl --source adf --duplex --pages 2 --multipage-out sheet.pdf
```

`--pages` is a limit in logical image sides and must be between 1 and 1000. Duplex requires `--source adf` and an even limit, so two sides represent one sheet. The adapter keeps one native feeder session or job open and emits sides in device order. Feeder exhaustion after at least one complete simplex page or duplex pair is a successful partial batch; an empty feeder or incomplete duplex pair is an error.

Use `--format` for individual page files. `--multipage-out` selects PDF or TIFF from its extension. A final multipage container is assembled only after acquisition succeeds; page images already emitted before a later error remain in the output directory. JPEG XL output requires the optional `cjxl` executable. See [backend requirements](backends.md) for tool requirements.

An eSCL id selected from `devices`, or one matching `OPEN_SCANLINE_ESCL_HOSTS`, uses the secure default open policy. A strict direct `escl:` id that is not in either set requires `--allow-unlisted-escl` on `scan` or `batch`. This is an explicit trust decision for that exact scheme, host, and port; it does not enable redirects or environment proxies.

## PDF and scanner-profile export

`scan`, `process`, and `batch` share runtime PDF/profile controls:

```bash
cargo run -- process --in scan.png --out searchable.pdf \
  --pdf-searchable --ocr-engine offline --ocr-lang eng
printf '%s\n' "$PDF_PASSWORD" | cargo run -- scan --device mock --out private.pdf \
  --pdf-password-file - --scanner-profile scanner_it8_profile.json
cargo run -- batch --device mock --out-dir pages --pages 2 \
  --multipage-out archive.pdf --pdf-searchable --ocr-engine tesseract --ocr-lang deu
```

Searchable and password options require a PDF destination. Scanner-profile JSON is validated before acquisition or output mutation and is applied to the final processed pixels before OCR. Tesseract remains an explicit optional dependency; `offline` uses the built-in engine.

For `scan`, `process`, and `batch`, omitted `--ocr-engine` and `--ocr-lang` values inherit `ocr_engine` and `ocr_language` from `config.json` (defaulting to `offline` and `eng`). Supplying either flag overrides only that setting for the current command.

The searchable layer uses 1% text opacity because commonly deployed PDF readers omit fully invisible text from extraction. It is normally imperceptible over the scanned page but can be faintly visible over very light content at high magnification.

PDF passwords are runtime-only and are never written to the JSON configuration.
Prefer `--pdf-password-file -` with standard input, as above. If a file is
required, create it outside the repository with permissions limited to the
current user, then remove it immediately after use:

```bash
password_file=$(mktemp)
trap 'rm -f "$password_file"' EXIT
chmod 600 "$password_file"
printf '%s\n' "$PDF_PASSWORD" > "$password_file"
cargo run -- process --in scan.png --out private.pdf \
  --pdf-password-file "$password_file"
```

Password input is limited to 4 KiB, must be nonempty valid UTF-8 without NUL bytes, and removes one trailing line ending. PDF encryption applies the PDF 2.0 SASLprep normalization and accepts at most 127 resulting UTF-8 bytes; longer or invalid normalized passwords are rejected rather than truncated. `--pdf-password` remains only for compatibility and is unsafe/deprecated: it requires `--allow-insecure-password-argv`, because command-line values can be retained in shell history or exposed to local process inspection. Neither password source is logged.

## OCR

Offline OCR is always available and uses the built-in template recognizer:

```bash
cargo run -- ocr --in processed.png --offline
```

Without `--offline`, Open Scanline requires `tesseract` on `PATH`. A missing executable, language pack, or failed Tesseract process is an explicit error; the application does not silently change engines. OCR results are printed as JSON.

## User ONNX inference

Run a local ONNX image model with the bundled pure-Rust CPU runtime. The command prints the inference report as JSON and never changes the input image:

```bash
cargo run -- onnx --in processed.png --model classifier.onnx --layout auto --normalization zero-to-one
```

Use `--input-name` for a model with multiple inputs. `--layout` accepts `auto`, `nchw`, or `nhwc`; `--normalization` accepts `zero-to-one` or `none`.

Open Scanline runs the model in a supervised worker process. It rejects model files above 64 MiB, batches above 16, image inputs above 16 million elements, oversized declared or intermediate tensors, external tensor data, and graphs above the documented node and output limits. The parent enforces a 30-second wall timeout and kills and reaps a worker that exceeds it. The worker also limits CPU threads and applies a 2 GiB process-memory ceiling on Linux and Windows. On macOS the parent samples resident memory and kills a worker above the same ceiling; a short allocation spike can occur between samples.

This boundary contains crashes, timeouts, and excessive resource use; it is not a filesystem or syscall sandbox. The worker runs with the same user account and can process arbitrary operators supported by `tract`, so treat models as executable local workloads. Library calls discover a sibling `open-scanline` worker and verify its private protocol version. If a host does not ship that sibling executable, call `run_user_onnx_with_worker` with an explicit version-matched Open Scanline worker; the default call fails without respawning an unrelated host executable.

## Portable package

Build the application first, then package the existing executable. The command checks the source metadata and verifies the archived copy by size and SHA-256 hash without executing the supplied program:

```bash
cargo build --release --no-default-features --locked
cargo run --release --no-default-features --locked -- package \
  --binary target/release/open-scanline --out open-scanline-portable.zip
```

On Windows, use `target/release/open-scanline.exe` for `--binary`.

`PORTABLE.txt` records `packaging_host_os` and `packaging_host_arch`. The packager validates a stable snapshot of the supplied file but does not infer a foreign binary's target triple, so create distribution archives on their intended target host or CI runner.

## Configuration and diagnostics

The default configuration file is in the platform-specific `open-scanline` configuration directory. Config and scanner-profile files are published atomically so a failed replacement does not truncate the previous file. Inspect the config or use an explicit path:

```bash
cargo run -- config --show
cargo run -- --config ./config.json config --init
cargo run -- info --module all
```

The `info` command reports compiled and currently available capabilities. Availability can change with the operating system, scanner drivers, network state, and optional executables.

## Desktop and host modes

Run the desktop interface with the default feature set:

```bash
cargo run -- gui
```

The `plugin` command provides a headless status and acquisition entry point for host software. It is not a native TWAIN acquisition interface. See [backend requirements](backends.md#twain-host-integration) before connecting it to a TWAIN workflow.
