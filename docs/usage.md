# Using Open Scanline

`open-scanline --help` lists the available commands. The examples below use
`cargo run --` while you are developing; drop that prefix once you have an
installed binary. For a core-only build use `--no-default-features`, or
`--no-default-features --features ocrs,onnx` to keep OCRS and ONNX without the
desktop GUI.

| I want to… | Start here |
| --- | --- |
| Scan a page or a file | [Scan and process an image](#scan-and-process-an-image) |
| Scan several pages from a feeder | [Batch output](#batch-output) |
| Make a searchable or password-protected PDF | [PDF and scanner-profile export](#pdf-and-scanner-profile-export) |
| Read the text on an image | [OCR](#ocr) |
| Run my own ONNX model | [User ONNX inference](#user-onnx-inference) |
| Build a portable release archive | [Portable package](#portable-package) |
| Check or change settings | [Configuration and diagnostics](#configuration-and-diagnostics) |
| Drive the GUI or a host program | [Desktop and host modes](#desktop-and-host-modes) |

## Scan and process an image

The mock source produces deterministic images, so it needs no hardware:

```bash
cargo run -- scan --device mock --out scan.png --width 320 --height 240 --dpi 150
cargo run -- process --in scan.png --out processed.png --rotate 90 --auto-levels
```

### Scan area

By default a physical scanner scans its full area. `--paper` limits the scan to
`a4`, `a5`, `a6`, `letter`, or `legal` (portrait). `--paper auto` ignores a
configured `paper_size` and uses the configured `default_width`/`default_height`,
which are `0` (the device maximum) unless you changed them. `--width` and `--height` are pixels at `--dpi` and override
`--paper` per axis, so `--width 1240 --dpi 150` is about 210 mm wide. The mock
source uses 320 x 240 pixels when no size is given.

```bash
cargo run -- scan --device 'sane:...' --out page.png --paper a4 --dpi 300
cargo run -- config --set-paper a4
```

The configuration stores the named area as `paper_size` (`auto` by default).
`default_width` and `default_height` are pixels, and `0` means the device
maximum. Configs written before `config_version` existed lose the old 320 x 240
default on load.

To scan from a local image instead, pass a `file:` device identifier:

```bash
cargo run -- scan --device file:/absolute/path/to/source.png --out copy.png
```

For a `file:` source, `--width`/`--height` (or `--paper`) form a bounding box: the
image is scaled to fit inside it and keeps its aspect ratio. With one axis the
image is scaled to that axis; with neither it keeps its size.

### Existing files and permissions

`scan`, `batch`, `process`, and `convert` never replace an existing output file
by default. They stop before scanning with
`output exists: <path>; pass --overwrite to replace it` (exit 1); pass
`--overwrite` to replace files. The desktop app and plugin mode manage their own
file names and still replace files. New files get your normal permissions (for
example `0644` with umask `022`); a replaced file keeps its mode.

`--crop x,y,w,h` needs a positive width and height (exit 2 otherwise). A crop
outside the scan area is an error that names the crop and the scan size. The
desktop app applies the same check before it starts a scan or batch.

Run `devices` to see the sources and backends available before you pick WIA, SANE,
or eSCL hardware:

```bash
cargo run -- devices
```

`--source` selects `flatbed`, `adf`, or `film`. When a backend supports regions,
a crop is forwarded as the acquisition region. A single `scan` always produces
one image, so `scan --duplex` fails with a hint to use `batch`; it never discards
the back side silently.

Color and processing controls are available on `scan`, `batch`, and `process`:

```bash
cargo run -- process --in scan.png --out corrected.png \
  --saturation 18 --hue -6 --curves 0:0,96:112,192:210,255:255
```

Curve coordinates must be in 0 through 255, and their x coordinates must increase
strictly. `--rotate` takes multiples of 90 degrees, `--deskew` an angle from -45
to 45 degrees (positive turns the page clockwise), and `--levels-gamma` a value above 0;
anything else, including `nan` or `inf`, exits 2. Likewise `--brightness` and `--contrast` take
-100 to 100, `--sharpen` 0 to 5, `--descreen-dpi` 25 to 300, and `--levels-black` must be below
`--levels-white` (0 to 255); `--grain-reduction` and `--infrared-clean` take `off`, `light`,
`medium` or `heavy` (older spellings such as `low`, `high` and `true` still work), and
`--colorize-mode` takes `off`, `sepia` or `blue` (the desktop app's `warm` and `cool` mean the
same). Config values follow the same limits, except that a saved `descreen_dpi` up to 1200 is
still accepted (descreening never uses more than 300), and `default_width` and
`default_height` must be set together or not at all. A configured `default_width` and
`default_height` are pixels at `default_dpi`; a different `--dpi` scales them. The same limits
apply to `rotate`, `deskew_angle`, and
`levels_gamma` in `config.json`, and a configured `film_type` must name a known
profile (or be empty, `off`, or `none`). An unknown `--film-type` is rejected
before the scanner starts. `--quality` applies to JPEG and JPEG XL output; for
other formats it is ignored with a warning.

`scan`, `batch`, and `process` start from the selected JSON configuration, and any
value you pass replaces the configured one. Configuration-aware switches accept
the bare form to enable a step (`--auto-levels`) and `=false` to disable one
(`--auto-levels=false`, `--duplex=false`); leaving a switch out keeps the
configured value. Use `off` or `none` to clear configured infrared-clean and
grain-reduction tiers, film type, and colorization mode, for example
`--infrared-clean=off` or `--film-type none`.

## Batch output

The batch command writes individual pages and can assemble a multipage TIFF or
PDF:

```bash
cargo run -- batch --device mock --out-dir pages --pages 3 --multipage-pdf document.pdf
cargo run -- batch --device mock --out-dir pages --pages 3 --multipage-out document.tif
cargo run -- batch --device 'escl:https@scanner.local:443' --out-dir sides \
  --allow-unlisted-escl --source adf --duplex --pages 2 --multipage-out sheet.pdf
```

`--pages` limits logical image sides and must be between 1 and 1000. Duplex
requires `--source adf` and an even limit, so two sides represent one sheet. The
adapter keeps one native feeder session or job open and emits sides in device
order. If the feeder empties after at least one complete simplex page or duplex
pair, the batch succeeds with fewer pages. If a SANE or WIA batch fails for
another reason, such as a jam, the pages already scanned are kept and the error
says how many were saved; the same holds when the final PDF or TIFF cannot be
written. A SANE batch saves each side as soon as `scanimage` finishes it, so
cancelling keeps the sides already saved (a WIA batch reads its sides when the
job ends). On a scanner that only has a feeder, a batch without
`--source` already runs as one feeder job; duplex still needs `--source adf`. An
empty feeder or an incomplete
duplex pair is an error.

Use `--format` for individual page files. `--multipage-out` chooses PDF or TIFF
from its extension. The final container is assembled only after acquisition
succeeds, so page images already written before a later error stay in the output
directory. JPEG XL output requires the optional `cjxl` executable. See
[backend requirements](backends.md) for tool requirements.

Page files are named `page_001`, `page_002`, and so on; a batch of 1,000 sides
uses four digits (`page_0001` to `page_1000`) so the names sort in page order.
PNG, JPEG, and TIFF files record the scan resolution; BMP, WebP, and GIF carry
none. In PDFs, grey pages are stored losslessly as greyscale and colour pages as
JPEG at quality 90. TIFFs use Deflate compression and switch to BigTIFF when the
pages would pass 4 GiB. One PDF holds at most 256 MiB of page images, so a very
long colour batch can still fail at the document step; its page files remain.

An eSCL id selected from `devices`, or one matching `OPEN_SCANLINE_ESCL_HOSTS`,
uses the secure default open policy. A strict direct `escl:` id that is in
neither set requires `--allow-unlisted-escl` on `scan` or `batch`. That opt-in is
an explicit trust decision for the exact scheme, host, and port; it does not
enable redirects or environment proxies.

When you request several aggregate outputs, they are produced in this order:
multipage alias, named TIFF, named PDF, then contact sheet. They share up to
512 MiB of private temporary decoded-page cache. The cache stores the published
page files, so any JPEG compression is reflected in every derivative. Changed
sources, cache errors, and exhausted capacity fall back to ordinary decoding.
Temporary cache files are cleaned up automatically, and completed outputs remain
if a later output fails or is cancelled.

## PDF and scanner-profile export

`scan`, `process`, and `batch` share the runtime PDF and profile controls:

```bash
cargo run -- process --in scan.png --out searchable.pdf \
  --pdf-searchable --ocr-engine offline --ocr-lang eng
printf '%s\n' "$PDF_PASSWORD" | cargo run -- scan --device mock --out private.pdf \
  --pdf-password-file - --scanner-profile scanner_it8_profile.json
cargo run -- batch --device mock --out-dir pages --pages 2 \
  --multipage-out archive.pdf --pdf-searchable --ocr-engine tesseract --ocr-lang deu
cargo run -- process --in scan.png --out latin-searchable.pdf \
  --pdf-searchable --ocr-engine ocrs --ocr-lang eng
```

Searchable and password options require a PDF destination. Scanner-profile JSON
is validated before acquisition or output changes, and is applied to the final
processed pixels before OCR. `offline` uses the built-in 5x7 compatibility
engine, `ocrs` uses an installed local model pack, and `tesseract` invokes the
optional system executable. A page on which the engine finds no text gets no
text layer rather than a placeholder.

For `scan`, `process`, and `batch`, omitted `--ocr-engine` and `--ocr-lang` values
inherit `ocr_engine` and `ocr_language` from `config.json` (defaulting to
`offline` and `eng`). Supplying either flag overrides only that setting for the
current command.

The searchable text layer uses 1% opacity because commonly deployed PDF readers
omit fully invisible text from extraction. It is normally imperceptible over the
scanned page, but can be faintly visible over very light content at high
magnification.

PDF passwords are runtime-only and are never written to the JSON configuration.
Prefer `--pdf-password-file -` with standard input, as above. If you must use a
file, create it outside the repository with permissions limited to your user, and
remove it immediately after use:

```bash
password_file=$(mktemp)
trap 'rm -f "$password_file"' EXIT
chmod 600 "$password_file"
printf '%s\n' "$PDF_PASSWORD" > "$password_file"
cargo run -- process --in scan.png --out private.pdf \
  --pdf-password-file "$password_file"
```

Password input is limited to 4 KiB, must be nonempty valid UTF-8 without NUL
bytes, and has one trailing line ending removed. PDF encryption applies PDF 2.0
SASLprep normalization and accepts at most 127 resulting UTF-8 bytes; a longer or
invalid normalized password is rejected rather than truncated. `--pdf-password`
remains only for compatibility and is unsafe and deprecated: it requires
`--allow-insecure-password-argv`, because command-line values can be retained in
shell history or exposed to local process inspection. Neither password source is
logged.

## OCR

Offline OCR is always available and uses the built-in 5x7 template recognizer.
It reads only clean, upright, uppercase block glyphs similar to its templates;
on ordinary scanned documents it returns little or garbled text. Use `tesseract`
(for example `brew install tesseract` on macOS) or an OCRS model pack for real
documents:

```bash
cargo run -- ocr --in processed.png --engine offline
```

`--offline` is a deprecated alias for `--engine offline` and cannot be combined
with `--engine`. Direct OCR defaults to Tesseract for compatibility. A missing
executable, language pack, or model pack, or a failed OCR process, is an explicit
error; the application does not silently switch engines. OCR results are printed
as JSON.

OCRS is an early-preview local engine for printed Latin documents. This release
supports only the `eng` language selector and requires separate RTen detection
and recognition models that you supply. Open Scanline never downloads or bundles
model weights. Install and inspect a model pack with:

```bash
cargo run -- ocr-model install \
  --detection /path/to/text-detection.rten \
  --recognition /path/to/text-recognition.rten
cargo run -- ocr-model status
cargo run -- ocr --in processed.png --engine ocrs --lang eng
```

Installation accepts regular files no larger than 64 MiB each, hashes and copies
them into an immutable pack in the platform-local application data directory,
verifies the copied hashes, and builds the OCR engine before atomically updating
`active.json`. Older packs are retained. These checks prove that the copied files
are intact and compatible with the selected OCRS release; they do not
authenticate the model publisher. OCRS does not expose recognition confidence, so
its compatibility `confidence` value is `0.0` and the JSON also reports
`confidence_available: false`.

## User ONNX inference

Run a local ONNX image model with the bundled pure-Rust CPU runtime. The command
prints the inference report as JSON and never changes the input image:

```bash
cargo run -- onnx --in processed.png --model classifier.onnx --layout auto --normalization zero-to-one
```

Use `--input-name` for a model with multiple inputs. `--layout` accepts `auto`,
`nchw`, or `nhwc`, and `--normalization` accepts `zero-to-one` or `none`.

Open Scanline runs the model in a supervised worker process. It rejects model
files above 64 MiB, batches above 16, image inputs above 16 million elements,
oversized declared or intermediate tensors, external tensor data, and graphs
above the documented node and output limits. The parent enforces a 30-second wall
timeout and kills and reaps a worker that exceeds it. The worker also limits CPU
threads and applies a 2 GiB process-memory ceiling on Linux and Windows. On macOS
the parent samples resident memory and kills a worker above the same ceiling; a
short allocation spike can occur between samples.

This boundary contains crashes, timeouts, and excessive resource use; it is not a
filesystem or syscall sandbox. The worker runs under your user account and can
process any operator `tract` supports, so treat models as executable local
workloads.

- Library callers must select and trust a version-matched Open Scanline executable
  explicitly with `OnnxRuntime::from_worker`. Content-identity and protocol checks
  reject a later replacement, but do not authenticate the publisher.
- The runtime executes a private read-only copy of the validated bytes, so
  replacing the pathname after the final check cannot substitute a different
  worker image. `run_user_onnx_with_worker` gives one call the same behavior.
- The former no-worker helpers `run_user_onnx` and `run_user_onnx_with_options`
  were removed; there is no automatic worker discovery.
- The CLI trusts only its own current-executable path, and still performs the
  worker identity, version, replacement, and containment checks.

## Portable package

Build the application first, then package the existing executable. The command
checks the source metadata and verifies the archived copy by size and SHA-256
hash without executing the supplied program:

```bash
cargo build --release --no-default-features --locked
cargo run --release --no-default-features --locked -- package \
  --binary target/release/open-scanline --out open-scanline-portable.zip
```

On Windows, use `target/release/open-scanline.exe` for `--binary`.

`PORTABLE.txt` records `packaging_host_os` and `packaging_host_arch`. The packager
validates a stable snapshot of the supplied file but does not infer a foreign
binary's target triple, so create distribution archives on their intended target
host or CI runner.

See [Portable archive](../README.md#portable-archive) for extraction and launcher
commands. Those instructions live in the README because the packager includes it
in every archive.

## Configuration and diagnostics

The default configuration file is `config.json` in the platform-specific
`open-scanline` configuration directory. A missing default file means the
built-in defaults are used. The global `--config PATH` option selects another file
and must appear before the subcommand. Config and scanner-profile files are
published atomically, so a failed replacement does not truncate the previous
file. Inspect the effective path and values, or initialize an explicit file:

```bash
cargo run -- config --show
cargo run -- --config ./config.json config --init
cargo run -- info --module all
```

`config` and `config --show` print the effective, validated configuration as
JSON on standard output and the file path (or a "file not found; showing
defaults" note) on standard error. An invalid file exits 1 with the reason.

For a bug report, add the global `--verbose` flag (before the subcommand), or set
`OPEN_SCANLINE_DEBUG=1`, to print backend diagnostics on standard error: which
tools were found, each external command with its exit code and error output, and
network errors. Timeouts can be raised with environment variables, in whole
seconds:

| Variable | Default | Range | Applies to |
| --- | --- | --- | --- |
| `OPEN_SCANLINE_SCAN_TIMEOUT_SECS` | 90 (single scan), 45 per batch side | 1–3600 | SANE and WIA acquisition |
| `OPEN_SCANLINE_PROBE_TIMEOUT_SECS` | 2–10 (availability), 8 (listing, option inspection) | 1–3600 | SANE and WIA discovery |
| `OPEN_SCANLINE_ESCL_PAGE_TIMEOUT_SECS` | 60 | 1–600 | each eSCL page |
| `OPEN_SCANLINE_ESCL_HEADER_TIMEOUT_SECS` | 5 | 1–600 | each eSCL response header |

Invalid or out-of-range values are ignored. `OPEN_SCANLINE_SIMULATE_BACKENDS=1`
lists and opens the simulated `sane:sim`, `wia:sim`, and `escl:sim` devices for
testing; without it, those ids refer to real devices.

The `info` command reports compiled and currently available capabilities.
Availability can change with the operating system, scanner drivers, network
state, and optional executables.

## Desktop and host modes

Run the desktop interface with the default feature set:

```bash
cargo run -- gui
```

The desktop opens a light scan workspace with scanner and file settings shown
together. Select single-image or multiple-side capture, check the list of files
the scan will write, and start the scan from the action bar at the bottom of the
window; if Start is unavailable, the bar says which setting to fix. The
progress and result views list published page files as they are written,
distinguish them from pending document outputs, and state why capture stopped.
Cancelling a batch can leave completed page files without a final PDF or TIFF.
Scanner maintenance (calibrate, focus, brightness) runs in the background, and
Start stays disabled while scanners are still being listed. A saved scanner that
is not found stays selected and is marked as not found instead of switching to
the mock source. Large images are previewed at reduced size. The main workspace
is English for now; the Image tools panels follow the configured language.

Without a configured `output_dir`, the desktop app saves to
`Pictures/Open Scanline` in your home folder (Documents, then the home folder,
if Pictures does not exist). The Scan area setting offers the device maximum,
named paper sizes, or a custom pixel size.

File names keep the existing naming rules: a scan uses
`<name>_scan_<frame>.<format>`, and batch page images are written under
`<output directory>/batch`. Image tools provides the image preview, histogram,
processing controls, Save/Save+, OCR, and configuration. The header's More menu
keeps scanner maintenance, profiles, preview scanning, and the existing menus.
The layout stacks vertically in narrower windows.

The `plugin` command provides a headless JSON status and acquisition entry point
for host software. It does not require the GUI feature:

```bash
cargo run --no-default-features -- plugin
cargo run --no-default-features -- plugin --device mock --out plugin-scan.png
```

Status output includes the application version, configuration path, platform,
device list, backend availability, and an `ok` field. With `--out`, plugin mode
performs one scan through the shared scan workflow and adds the selected device,
output path, and byte count. Without `--device` it scans the configured
`last_device_id` (the built-in mock source if none is set) with the configured
processing settings. A configuration that cannot be loaded makes the status
report `ok: false` with an `error` field and exit 1. `--quiet` suppresses JSON output. Success exits 0,
ordinary failures exit 1, invalid CLI arguments exit 2, and an interrupted plugin
operation exits 130.

Plugin mode is a local process contract, not a network API or a native TWAIN
acquisition interface. It may enumerate installed tools and reachable devices, so
apply the same backend trust rules as other commands. See
[backend requirements](backends.md#twain-host-integration) before connecting it to
a TWAIN-capable host or bridge.
