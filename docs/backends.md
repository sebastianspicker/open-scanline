# Backend and tool requirements

Open Scanline always ships mock and file-image sources, so you can build and try
the workflow without a scanner. Real hardware depends on your operating
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
| SANE | A working `scanimage` (on `PATH`, or in Homebrew/MacPorts on macOS) and a configured SANE device | Capabilities and acquisition are delegated to the installed SANE backend. See [SANE on macOS](#sane-on-macos). |
| eSCL | Network access to an AirScan/eSCL-compatible scanner | Explicit hosts, local mDNS discovery, and bounded subnet discovery over HTTP or HTTPS. |

### WIA

WIA is Windows-only. Enumeration filters out imaging devices that are not
scanners. A transfer sets the color intent (the CLI and desktop app request RGB
color), independent x/y resolution, offsets,
extents, flatbed/feeder/film item categories, and the advertised feeder duplex
controls. An ADF batch keeps one PowerShell process and one COM device connection
open, asks for a bounded number of logical sides, and decodes the numbered outputs
in order.

Real WIA calibration and focus are reported as unsupported. The standard WIA
scanner property surface exposes neither operation, and Open Scanline does not
repurpose camera-focus properties to fake them.

WIA runs `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` (the bare
`powershell` only if that file is missing) and recognizes an empty feeder by the
`WIA_ERROR_PAPER_EMPTY` result code as well as by message text. WIA property
availability and feeder-empty behavior vary by driver, so validate the workflow
with real Windows hardware.

### SANE

SANE needs `scanimage`; `sane-find-scanner` on its own is not enough. The adapter
parses the device's reported source, color mode, duplex, and resolution
constraints, ignores options that `scanimage` marks `[inactive]`, picks the
nearest advertised discrete or ranged resolution, and rejects a source or color
request it cannot match. If `scanimage --help -d <device>` fails or lists no
options, the scan stops with an error instead of guessing option values. A document
batch uses one `scanimage --batch` process and reads the numbered page files in
numeric order.

Without `--paper`, `--width`, or `--height`, Open Scanline sends no scan-area
options and the backend scans its full default area. A named paper size is sent
as `-x`/`-y` in millimetres; sheet-fed backends that advertise `--page-width` and
`--page-height` (for example `canon_dr` and `fujitsu`) receive those first. The
image keeps the pixel size the scanner delivers. When the default flatbed source
is requested from a device whose sources are all feeders, its single-sided
feeder (for example `ADF Front`) is used; any other unavailable source is an
error that lists what the device offers. `scanimage` runs in the C locale, and a
batch that stops because the feeder is empty (exit status 7) keeps the pages
already scanned.

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

### SANE on macOS

macOS has no built-in SANE. Install the backends with Homebrew and check that
SANE itself sees the scanner before you use Open Scanline:

```bash
brew install sane-backends
scanimage -L
```

`scanimage -L` must print a line such as
``device `canon_dr:libusb:001:004' is a CANON P-215II sheetfed scanner``. If it
prints "No scanners were identified", Open Scanline cannot use the device
either; check the cable, power, and the device notes below. Then:

```bash
open-scanline devices
open-scanline scan --device sane:canon_dr:libusb:001:004 --paper a4 --dpi 300 --out page.png
open-scanline batch --device sane:canon_dr:libusb:001:004 --source adf --duplex \
  --pages 4 --paper a4 --dpi 300 --out-dir pages --multipage-pdf pages.pdf
```

- The device id is `sane:` followed by the SANE name. A USB name such as
  `libusb:001:004` contains the bus and address and can change when you replug
  the scanner, so run `open-scanline devices` again after reconnecting.
- Feeder-only scanners use their front feeder automatically; pass
  `--source adf` to be explicit and `--duplex` (batch only) for both sides.
- `--paper` sets the physical page size. Without it the scanner's full default
  area is used, which for sheet-fed devices may be taller than your page.
- The desktop app finds `scanimage` in `/opt/homebrew/bin`, `/usr/local/bin`,
  and `/opt/local/bin` even when it is started from Finder or the Dock, where
  your shell's `PATH` does not apply.
- To see what the device supports, run `scanimage --help -d <sane name>`. For a
  bug report, rerun the failing command with `--verbose` and include its
  output.

Canon imageFORMULA P-215II: SANE's `canon_dr` backend lists this model (USB
`1083:165b`) with status "untested". Set the scanner's Auto Start
(CaptureOnTouch Lite) switch to off; with it on, the scanner presents itself as
a storage drive instead of a scanner and `scanimage -L` does not find it.

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
  CIDR and is capped at 32 probes (a prefix such as `192.168.1` therefore probes
  `.1`–`.32`; list other scanners in `OPEN_SCANLINE_ESCL_HOSTS`). Public, loopback, unspecified, multicast, and
  broadcast ranges are ignored; trusted non-local endpoints belong in
  `OPEN_SCANLINE_ESCL_HOSTS`.

Normal acquisition opens only an exact discovered id or an endpoint listed in
`OPEN_SCANLINE_ESCL_HOSTS`. `scan` and `batch` also accept `--allow-unlisted-escl`
for an explicit, strict `escl:` id that was neither discovered nor listed. That
opt-in trusts only the canonical endpoint; the client still disables environment
proxies and redirects. A scan-job `Location` that names another origin (for
example an IP address for a scanner found as `scanner.local`) is used only for its
`/<root>/ScanJobs/<id>` path on the same endpoint; another host is never
contacted, and the job id from the response body is the fallback.

HTTPS uses normal certificate verification. Most scanners present a
self-signed certificate, which that rejects. To trust one scanner, pin its
certificate's SHA-256 fingerprint (of the DER certificate; 64 hex digits, `:`
optional) for its host:

```bash
export OPEN_SCANLINE_ESCL_CERT_PINS='scanner.local=AB:CD:...,192.168.1.50:443=abcd...'
```

A pinned host accepts exactly that certificate. Chain, expiry, and name are not
checked for it, but the TLS handshake is still verified, so the scanner must hold
the certificate's key. Every other host keeps full verification. A host-only pin
covers all ports, a `host:port` pin takes precedence on its port, and a `.local`
name and its IP address need separate pins. To read the fingerprint, run the
failing command with `--verbose` (the log names the presented certificate's
SHA-256) and confirm it on the scanner's own status page before pinning it; a
wrong pin fails with "does not match the pinned SHA-256 fingerprint (got ...)".
Pinning an attacker's certificate lets that one scanner's traffic be read or
forged, so never pin a fingerprint you have not confirmed. A pin applies only to
HTTPS; it does not stop a discovered plain-HTTP endpoint for the same host from
being used, so select the `https` device id. The fingerprint log line needs
`--verbose` or `OPEN_SCANLINE_DEBUG=1` and costs one extra TLS handshake (no
request is sent).

Discovery probes run with bounded concurrency and one overall deadline, and
capability and document response bodies are size-limited. Prefer explicit device
URLs on large or filtered networks.

Resolutions come from `DiscreteResolution` pairs and `ResolutionRange` limits in
the capabilities; the client sends the nearest supported value (snapped to the
range step) and the requested value unchanged when the device advertises none.
`NextDocument` retries back off from 250 ms to 2 s within the page deadline and
honour a `Retry-After` in seconds.

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
| `tesseract` | OCR when the `tesseract` engine is selected | Install Tesseract and its requested language data, and make `tesseract` available on `PATH` (`brew install tesseract` on macOS). Missing or failed Tesseract runs return an error; select `--ocr-engine offline` or `ocrs` instead. Confidence is the mean word confidence from Tesseract's TSV output. |
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

Mock and file sources support hardware-free runs, but cannot prove physical WIA,
SANE, feeder, film-unit, or vendor eSCL compatibility.
