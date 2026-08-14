# Backend and tool requirements

Open Scanline always includes mock and file-image sources. Hardware access depends on the operating system, installed software, device permissions, and network conditions. Check the current machine with `open-scanline devices` and `open-scanline info --module all`.

## Scanner sources

| Source | Requirement | Notes |
| --- | --- | --- |
| Mock | None | Built in and suitable for deterministic repeated-page simulation. It does not emulate feeder exhaustion or hardware source negotiation. |
| File | A readable local image | Use `file:/path/to/image`; set `OPEN_SCANLINE_FILE_DEVICE` to list one file source through `devices`. Batch mode repeats the source image and does not emulate an ADF. |
| WIA | Windows, a working Windows PowerShell, and a usable WIA scanner | Scanner-only enumeration and transfers through Windows Image Acquisition. |
| SANE | A working `scanimage` on `PATH` and a configured SANE device | Capabilities and acquisition are delegated to the installed SANE backend. |
| eSCL | Network access to an AirScan/eSCL-compatible scanner | Explicit hosts, local mDNS discovery, and bounded subnet discovery over HTTP or HTTPS. |

### WIA

WIA enumeration filters out non-scanner imaging devices. A transfer maps color intent, independent x/y resolution, offsets, extents, flatbed/feeder/film item categories, and advertised feeder duplex controls. An ADF batch keeps one PowerShell process and one COM device connection, requests a bounded number of logical sides, and decodes numbered outputs in order.

The Windows COM path is covered by deterministic command/adapter tests but is not exercised on macOS. WIA property availability and feeder-empty behavior vary by driver, so real Windows hardware remains the authority.

### SANE

The adapter requires `scanimage`; `sane-find-scanner` alone is not sufficient. It parses the device's reported source, color-mode, duplex, and resolution constraints, selects the nearest advertised discrete or ranged resolution, and rejects an explicit source or color request when those constraints cannot be interpreted safely. A document batch uses one `scanimage --batch` process and reads numbered page files in numeric order.

SANE option names, values, and feeder-empty messages remain backend-specific. Unsupported requested sources fail explicitly. Real SANE hardware and vendor backends must be verified on the machine that will scan.

### eSCL

The client reads `ScannerCapabilities` and negotiates the advertised capability root, platen/ADF/film source, simplex or duplex ADF, color mode, x/y resolution, physical scan region, and a decodable PNG, JPEG, or TIFF document format. It creates one scan job and streams repeated `NextDocument` responses until the requested side limit or feeder exhaustion, deleting an unfinished job after cancellation, callback failure, or a reached limit.

PDF-only and unknown response formats are rejected rather than treated as raster images. Vendors differ in capability XML and terminal job status, so the parser deliberately supports the bounded common fields and rejects unsupported combinations.

eSCL discovery can be disabled with `OPEN_SCANLINE_NETWORK_DISCOVERY=0`. Set `OPEN_SCANLINE_ESCL_HOSTS` to a comma-separated list of known hosts when explicit discovery is preferred. `OPEN_SCANLINE_ESCL_SUBNETS` accepts a private or link-local IPv4 host prefix or CIDR and is capped to 64 probes. Public, loopback, unspecified, multicast, and broadcast ranges are ignored; explicitly trusted non-local endpoints belong in `OPEN_SCANLINE_ESCL_HOSTS`.

Normal acquisition opens only an exact discovered id or an endpoint in `OPEN_SCANLINE_ESCL_HOSTS`. `scan` and `batch` accept `--allow-unlisted-escl` for an explicit, strict `escl:` id when discovery is unavailable. The opt-in trusts only that canonical endpoint; the client still disables environment proxies and redirects and pins scan-job locations to the same origin.

Discovery probes run with bounded concurrency and one overall deadline. Explicit device URLs are preferable on large or filtered networks. Capability and document response bodies are size-limited.

All physical adapters reject resolutions below 50 dpi instead of silently changing the request. Devices may negotiate a different advertised resolution; eSCL keeps the requested physical region unchanged when it does so.

## TWAIN host integration

TWAIN is not a scanner backend in Open Scanline. The application ships no native TWAIN Data Source and does not perform TWAIN acquisition. Its TWAIN-related code launches the headless `plugin` mode so that a separate TWAIN-capable host or bridge can coordinate Open Scanline. Install, configure, and trust that host or bridge independently.

## Optional executables

| Tool | Used for | Requirement |
| --- | --- | --- |
| `scanimage` | SANE enumeration and acquisition | Install SANE and make `scanimage` available on `PATH`. |
| `tesseract` | OCR when `--offline` is not supplied | Install Tesseract and its requested language data, and make `tesseract` available on `PATH`. Missing or failed Tesseract runs return an error; use `--offline` to select the built-in recognizer. |
| `cjxl` | JPEG XL output | Install libjxl's `cjxl` executable and make it available on `PATH`. Other documented image outputs do not require it. |

The desktop GUI requires the default `gui` Cargo feature and a usable desktop session. Build the core without it with `cargo build --no-default-features`.

Automated tests use simulation, injected command runners, and local HTTP/TLS fixtures. They do not prove physical WIA, SANE, feeder, film-unit, or vendor eSCL compatibility.
