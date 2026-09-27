# Backend and tool requirements

Open Scanline always ships mock and file-image sources, so you can build, test,
and try the workflow without a scanner. Real hardware depends on your operating
system, installed software, device permissions, and network. Check what the
current machine sees with:

```bash
open-scanline devices
open-scanline info --module all
```

## Scanner sources

| Source | Requirement | Notes |
| --- | --- | --- |
| Mock | None | Built in and suitable for deterministic repeated-page simulation. It does not emulate feeder exhaustion or hardware source negotiation. |
| File | A readable local image | Use `file:/path/to/image`; set `OPEN_SCANLINE_FILE_DEVICE` to list one file source through `devices`. Batch mode repeats the source image and does not emulate an ADF. |
| WIA | Windows, a working Windows PowerShell, and a usable WIA scanner | Scanner-only enumeration and transfers through Windows Image Acquisition. |
| SANE | A working `scanimage` on `PATH` and a configured SANE device | Capabilities and acquisition are delegated to the installed SANE backend. |
| eSCL | Network access to an AirScan/eSCL-compatible scanner | Explicit hosts, local mDNS discovery, and bounded subnet discovery over HTTP or HTTPS. |

### WIA

WIA is Windows-only. Enumeration filters out imaging devices that are not
scanners. A transfer maps your color intent, independent x/y resolution, offsets,
extents, flatbed/feeder/film item categories, and the advertised feeder duplex
controls. An ADF batch keeps one PowerShell process and one COM device connection
open, asks for a bounded number of logical sides, and decodes the numbered outputs
in order.

Real WIA calibration and focus are reported as unsupported. The standard WIA
scanner property surface exposes neither operation, and Open Scanline does not
repurpose camera-focus properties to fake them. The test-only WIA simulation
labels its synthetic calibration and point focus as simulated.

The Windows COM path has deterministic command and adapter tests, but it is not
exercised on macOS. WIA property availability and feeder-empty behavior vary by
driver, so real Windows hardware remains the authority.

### SANE

SANE needs `scanimage`; `sane-find-scanner` on its own is not enough. The adapter
parses the device's reported source, color mode, duplex, and resolution
constraints, picks the nearest advertised discrete or ranged resolution, and
rejects an explicit source or color request it cannot interpret safely. A document
batch uses one `scanimage --batch` process and reads the numbered page files in
numeric order.

Maintenance support is discovered per device with `scanimage -A` and cached for
the selected GUI device. Open Scanline recognizes only three shapes:

- an argument-free `--calibrate` button,
- `--autofocus` or `--focus-on-centre` for centre focus,
- finite numeric `--focusx` and `--focusy` ranges combined with `--autofocus` for
  point focus.

It never copies arbitrary option names or descriptive text into a command.
Maintenance passes the selected device identifier as a separate argument, adds
`--dont-scan`, and reuses the existing cancellation and process timeout controls.
A centre-only device rejects point requests instead of quietly changing their
meaning.

SANE option names, values, and feeder-empty messages stay backend-specific, and an
unsupported requested source fails explicitly. Verify real SANE hardware and
vendor backends on the machine that will scan.

Mock maintenance is explicitly simulated. File and eSCL sources, and the SANE
simulation device, report maintenance as unsupported.

### eSCL

The client reads `ScannerCapabilities` and negotiates the advertised capability
root, platen/ADF/film source, simplex or duplex ADF, color mode, x/y resolution,
physical scan region, and a decodable PNG, JPEG, or TIFF document format. It opens
one scan job and streams `NextDocument` responses until it reaches the requested
side limit or the feeder empties, deleting an unfinished job after cancellation, a
callback failure, or a reached limit.

PDF-only and unknown response formats are rejected rather than treated as raster
images. Vendors differ in their capability XML and terminal job status, so the
parser supports the bounded common fields and rejects unsupported combinations.

Discovery controls:

- `OPEN_SCANLINE_NETWORK_DISCOVERY=0` disables discovery entirely.
- `OPEN_SCANLINE_ESCL_HOSTS` takes a comma-separated list of known hosts.
- `OPEN_SCANLINE_ESCL_SUBNETS` takes a private or link-local IPv4 host prefix or
  CIDR and is capped at 64 probes. Public, loopback, unspecified, multicast, and
  broadcast ranges are ignored; trusted non-local endpoints belong in
  `OPEN_SCANLINE_ESCL_HOSTS`.

Normal acquisition opens only an exact discovered id or an endpoint listed in
`OPEN_SCANLINE_ESCL_HOSTS`. When discovery is unavailable, `scan` and `batch`
accept `--allow-unlisted-escl` for an explicit, strict `escl:` id. That opt-in
trusts only the canonical endpoint; the client still disables environment proxies
and redirects and pins scan-job locations to the same origin.

Discovery probes run with bounded concurrency and one overall deadline, and
capability and document response bodies are size-limited. Prefer explicit device
URLs on large or filtered networks.

Every physical adapter rejects resolutions below 50 dpi instead of silently
changing the request. A device may negotiate a different advertised resolution;
eSCL keeps the requested physical region unchanged when it does.

## TWAIN host integration

TWAIN is not a scanner backend here. Open Scanline ships no native TWAIN Data
Source and does not acquire through TWAIN. Its TWAIN-related code launches the
headless `plugin` mode so a separate TWAIN-capable host or bridge can coordinate
Open Scanline. Install, configure, and trust that host or bridge yourself.

## Optional executables

| Tool | Used for | Requirement |
| --- | --- | --- |
| `scanimage` | SANE enumeration and acquisition | Install SANE and make `scanimage` available on `PATH`. |
| `tesseract` | OCR when the `tesseract` engine is selected | Install Tesseract and its requested language data, and make `tesseract` available on `PATH`. Missing or failed Tesseract runs return an error; select `offline` for the built-in recognizer. |
| `cjxl` | JPEG XL output | Install libjxl's `cjxl` executable and make it available on `PATH`. Other documented image outputs do not require it. |

OCRS requires the `ocrs` Cargo feature (on by default), but it is not an
executable dependency. It also needs an operator-supplied local detection and
recognition model pack, installed with `ocr-model install`; only printed Latin
`eng` input is supported. Open Scanline does not bundle or download those weights.

The desktop GUI requires the `gui` Cargo feature and a usable desktop session.

## Build profiles

| Build | Cargo flags | Included support |
| --- | --- | --- |
| Default | none | GUI, OCRS, ONNX, core |
| Core | `--no-default-features` | Scan, processing, media, template OCR, Tesseract integration |
| Full headless | `--no-default-features --features ocrs,onnx` | Core plus OCRS and ONNX |
| Individual | `--no-default-features --features gui` (or `ocrs`, `onnx`) | Core plus the selected feature |

ONNX requires the `onnx` feature. Every profile keeps the public Rust types,
functions, and CLI arguments. Selecting an inference feature you did not compile
returns an explicit unsupported error before any inference artifact or model
install is created. Diagnostics report compiled support separately from installed
models, and features never install external executables or download models.

Automated tests use simulation, injected command runners, and local HTTP/TLS
servers. They cannot prove physical WIA, SANE, feeder, film-unit, or vendor eSCL
compatibility.
