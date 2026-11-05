# Architecture

Open Scanline is a native Rust application. A single library powers the CLI, the
optional desktop GUI, and the headless plugin and host entry points. The public
library paths are thin re-export modules over private layers.

## System context

Open Scanline is a layered monolith: one executable and one library target in a
single Cargo package. It stores settings and output artifacts locally, calls
installed scanner or media tools when you ask it to, and talks to eSCL scanners on
the local network. There is no server and no database.

This diagram answers "who calls Open Scanline, and what does it call in turn?":

```mermaid
flowchart LR
    user((User)) -->|subcommands| cli[CLI]
    user -->|clicks| gui[Desktop GUI]
    host[Host or bridge] -->|"runs plugin mode"| plugin[Plugin entry]
    lib[Library caller] -->|Rust calls| facades[Public facades]
    cli -->|typed request| workflows[Workflows]
    gui -->|"job thread"| workflows
    plugin -->|typed request| workflows
    facades -->|re-exports| workflows
    workflows -->|"open, scan, save"| infrastructure[Infrastructure adapters]
    workflows -->|pure functions| domain[Domain values and rules]
    infrastructure --> domain
    infrastructure -->|"exec, HTTP(S), files"| external[Scanners, tools, ONNX worker]
    classDef focus fill:#fde68a,stroke:#b45309,color:#1f2937
    class workflows focus
```

Every entry point ends in the same workflow functions, so a scan started from the
CLI, the GUI or the plugin mode runs the same validation, processing and
publication code. External programs (`scanimage`, PowerShell for WIA, Tesseract,
the JPEG XL encoder, the ONNX worker) are child processes; eSCL scanners are
reached over HTTP(S).

Where in the code:

- CLI: [src/main.rs](../src/main.rs) → [src/inbound/cli.rs](../src/inbound/cli.rs) → [src/inbound/cli/handlers/router.rs](../src/inbound/cli/handlers/router.rs)
- Desktop GUI: [src/inbound/gui/app/runner.rs](../src/inbound/gui/app/runner.rs)
- Plugin entry: [src/inbound/plugin.rs](../src/inbound/plugin.rs)
- Public facades: [src/lib.rs](../src/lib.rs) and the top-level `src/*.rs` modules
- Workflows: [src/workflows/](../src/workflows/)
- Infrastructure adapters: [src/infrastructure/](../src/infrastructure/)
- Domain: [src/domain/](../src/domain/)

## Layers

Dependencies point one way. A layer may use any layer below it, never one above:

```text
inbound  ->  workflows  ->  infrastructure  ->  domain
```

`cargo test` enforces this direction: the `architecture_rules` unit test fails
with `file:line` when `src/domain/`, `src/infrastructure/`, or `src/workflows/`
refers to a layer above it.

`src/domain/` holds values and rules with no I/O: the scan request and page-limit
rules, image buffers and geometry, processing preferences and the pure processing
operations, export options and OCR engine names, settings validation, and the
manufacturer catalog.

`src/infrastructure/` holds every concrete adapter. `acquisition` owns the
`DeviceSession` contract and its mock, file, SANE, WIA, and eSCL
implementations; `command_backend` is the shared machinery for the two
command-driven backends. `media` owns codecs, PDF, TIFF, OCR, ICC profiles, and
the aggregate page cache. `config` owns `AppConfig` and its JSON persistence.
`runtime` owns process supervision, command capture, atomic publication, and
temporary files; it depends on no other adapter. `onnx` owns the isolated
inference worker, and `distribution` the portable archive.

`src/workflows/` holds each multi-step use case and calls infrastructure
directly: `capture::single` and `capture::batch` (including batch output
naming), `process`, `publication` (validation, OCR, profile correction and
atomic output), `maintenance` (calibration, focus, exposure, and IT8 scanner
profiling), and `settings` (resolving configured defaults). Each use case has one
canonical entry that takes its arguments plus an options struct.
`workflows::compat` holds only the one-argument public entries that run the scan,
batch, and process use cases with default options; the facades export each
canonical entry as `*_with_options` with its public options struct.

`src/inbound/` translates external requests: CLI parsing and output, the optional
GUI, diagnostics, plugin mode, host integration, and UI translations (loaded from
`assets/i18n/`). Inbound calls workflows for anything that opens a device
session or publishes output through more than one adapter. It may call
infrastructure directly only for these single-step capabilities: device
inventory, config persistence, platform information, packaging, single-file
convert, single-image OCR and the OCRS model pack, and the ONNX command entries.

`src/error.rs` (`ScanError`) and `src/operation.rs` (`CancellationToken`) are
shared by every layer.

There are no traits whose only purpose is to separate layers. `DeviceSession` is a
trait because five backends implement it and library callers may implement it
too. The remaining traits are injection seams for a process or engine that
can be supplied independently: `CommandRunner` and `ImageDecoder` (SANE/WIA
commands and artifact decoding), `OcrsRunner`, and `ModelPairValidator`.
Everything else is a concrete type or function.

## State and data flow

An inbound adapter builds typed input and calls the canonical workflow entry.
Capture opens one device session, produces one image or a bounded stream of
logical sides, applies domain processing, and publishes through media. Batch
keeps page paths in order and assembles optional documents after the pages are
published, instead of holding every decoded page in memory. The inbound edge
loads and saves JSON settings through `infrastructure::config`; configuration and
language are application-instance state, while translation catalogs are immutable
data.

```mermaid
flowchart LR
    request[Typed scan or process request] --> validate[Validate request and output]
    validate --> session[Open one acquisition session]
    session --> pages[Acquire one image or bounded side stream]
    pages --> processing[Apply domain processing and profile correction]
    processing --> ocr[Run optional OCR]
    ocr --> publish[Publish each output atomically]
    publish --> container[Assemble optional multipage TIFF or PDF]
    container --> result[Return ordered output paths]
```

Scanner artifacts, OCR input, configuration, profiles, and package outputs are
validated before same-directory atomic publication. Cancellation flows through the
operation and into command-backed adapters.

### Where state lives

There is no database. Everything persistent is a file:

| Store | Location | Written by |
| --- | --- | --- |
| Settings (`AppConfig`) | `config.json` in the platform config directory, or `--config <path>` | [src/infrastructure/config/json.rs](../src/infrastructure/config/json.rs) |
| OCRS model packs | `ocr-models/` in the platform data directory | [src/infrastructure/media/ocr/model_pack.rs](../src/infrastructure/media/ocr/model_pack.rs) |
| Scan and process outputs | The path or directory the user names | [src/infrastructure/media/](../src/infrastructure/media/) via [src/workflows/publication.rs](../src/workflows/publication.rs) |
| Scanner artifacts, page cache | Private temporary directories, removed on drop | [src/infrastructure/runtime/temporary_output.rs](../src/infrastructure/runtime/temporary_output.rs), [src/infrastructure/media/aggregate_cache.rs](../src/infrastructure/media/aggregate_cache.rs) |

The platform directories come from
[src/infrastructure/runtime/platform.rs](../src/infrastructure/runtime/platform.rs)
via the `dirs` crate, each with an `open-scanline` subdirectory: `~/.config` and
`~/.local/share` on Linux, `~/Library/Application Support` for both on macOS, and
`AppData\Roaming` and `AppData\Local` on Windows.

## User journeys

### `open-scanline scan`

This sequence answers "what happens, file by file, when I run a single scan?":

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant cli as CLI handlers
    participant single as capture::single
    participant acq as acquisition
    participant page as workflows::process
    participant media as media
    U->>cli: scan --device mock --out scan.png
    cli->>cli: parse args, load_config
    cli->>single: run_scan_to_file(args, options)
    single->>single: validate output and request
    single->>acq: open_device_with_policy(id)
    acq-->>single: AnySession
    single->>acq: set_params, scan
    acq-->>single: ImageBuffer
    single->>page: process_and_publish_page
    page->>page: pipeline, profile, optional OCR
    page->>media: save image or PDF
    media-->>page: temp file renamed to --out
    page-->>single: published path
    single->>acq: close
    single-->>cli: Ok(path)
    cli-->>U: "wrote scan.png (N bytes)", exit 0
```

Steps 2–4 turn CLI flags and `config.json` into one `ScanToFileArgs`; explicit
flags win over configured defaults. Step 4 rejects bad output paths, crop
rectangles and single-image duplex *before* a device is opened (step 5), so a
mistake never costs a physical scan. The registry in step 5 picks the backend from
the device id prefix (`mock`, `file:`, `sane:`, `wia:`, `escl:`). Steps 9–13 are
shared with `process` and every batch page: the image is written to a temporary
file next to the destination, checked, and only then renamed into place. Errors
map to exit code 1, cancellation (Ctrl+C) to 130.

Where in the code:

- Steps 1–3: [src/inbound/cli/handlers/router/routes.rs](../src/inbound/cli/handlers/router/routes.rs) (`scan`), [src/inbound/cli/handlers/scan.rs](../src/inbound/cli/handlers/scan.rs) (`run`)
- Steps 4–8, 14: [src/workflows/capture/single.rs](../src/workflows/capture/single.rs) (`run_scan_to_file`, `ScanContext`)
- Steps 5–6: [src/infrastructure/acquisition/registry.rs](../src/infrastructure/acquisition/registry.rs) (`open_device_with_policy`); the session contract is in [src/infrastructure/acquisition/contract.rs](../src/infrastructure/acquisition/contract.rs)
- Steps 9–10: [src/workflows/process.rs](../src/workflows/process.rs) (`process_and_publish_page`), [src/domain/processing/](../src/domain/processing/)
- Steps 11–12: [src/workflows/publication.rs](../src/workflows/publication.rs) (`save_final_image_with_searchable_text`), [src/infrastructure/media/codecs.rs](../src/infrastructure/media/codecs.rs) (`save_standard_image_output`)

`batch` follows the same shape through
[src/workflows/capture/batch.rs](../src/workflows/capture/batch.rs)
(`run_batch_scan_with_report`): it publishes each side as it arrives, then
assembles the optional multipage TIFF or PDF from the published page files.

### Scan from the desktop GUI

This sequence answers "how does a button click reach the same workflow without
freezing the window?":

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant ui as GUI frame loop
    participant job as Job thread
    participant single as capture::single
    U->>ui: Click Scan
    ui->>ui: start_scan, scan_job_plan
    ui-)job: spawn_job(plan, cancel flag)
    job->>single: run_scan_to_file
    single--)job: progress callbacks
    job-)ui: GuiJobEvent::Progress over mpsc
    single-->>job: Ok(path)
    job-)ui: terminal GuiJobEvent
    ui->>ui: drain_job_events, show result
```

The GUI never scans on its render thread. It builds the same `ScanToFileArgs` the
CLI builds (step 2), runs the workflow on a worker thread (step 3), and polls an
`mpsc` channel on each frame (steps 6 and 8). Only one job runs at a time; the
Cancel button sets the shared flag that the workflow checks between phases.

Where in the code:

- Steps 1–3: [src/inbound/gui/app/acquire.rs](../src/inbound/gui/app/acquire.rs) (`start_scan`, `launch_scan_job`), [src/inbound/gui/app/scan.rs](../src/inbound/gui/app/scan.rs) (`scan_job_plan`)
- Step 3 thread and channel: [src/inbound/gui/app/jobs.rs](../src/inbound/gui/app/jobs.rs) (`spawn_job`)
- Steps 6–9: [src/inbound/gui/app/lifecycle.rs](../src/inbound/gui/app/lifecycle.rs) (`drain_job_events`)

## External contracts

`DeviceSession` is the acquisition-session contract. Its maintenance capability
query has a safe unsupported default, so an adapter must advertise calibration or
centre/point focus before inbound code can dispatch it. Mock and file backends are
deterministic, hardware-free sources; physical backends provide the same contract.
Batch limits count logical image sides, from 1 through 1,000, and a single-image
duplex request is rejected rather than losing a side.

SANE needs `scanimage`; a SANE batch publishes each side as soon as `scanimage`
renames it from its `.part` name, so the private artifact quota bounds only the
sides not yet published. WIA needs Windows PowerShell/COM and compatible hardware;
eSCL uses bounded HTTP(S) discovery or an explicit endpoint. An unlisted eSCL
endpoint requires the strict `--allow-unlisted-escl` opt-in. TWAIN support starts a
separate host integration — it is not a native TWAIN Data Source or an acquisition
backend.

SANE maintenance parses a separate device option inventory and builds only fixed
allowlisted commands. Real WIA, eSCL, and file maintenance stay unsupported; mock
maintenance is marked simulated. The GUI caches that snapshot on device selection
and inventory refresh, and disables any operation the selected device did not
advertise.

Template OCR is built in. OCRS uses an explicitly installed local RTen model pack,
while Tesseract OCR and JPEG XL depend on explicitly selected optional executables.
User ONNX models run locally in a supervised, explicitly selected, version-matched
Open Scanline worker with resource limits — not in a filesystem or syscall
sandbox. There is no worker discovery: library callers always name the worker
executable.

The CLI (subcommands, output, exit codes 0/1/2/130), the plugin status JSON, the
configuration file, scanner-profile JSON, the OCRS model-pack manifest, and the
portable archive layout are external contracts. The CI build also exercises the
portable archive launcher.

A successful build cannot prove a physical scanner, driver, feeder, firmware,
desktop session, optional model pack, or optional tool on a target machine.

There is no deployment service and no database migration boundary. Distribution is
an explicitly invoked portable-ZIP step around an already built binary, and
`Cargo.toml` disables crate publication. Build and package on the intended target
host, because the packager records — but does not infer — the executable's target
operating system and architecture.

## Build and CI

This pipeline answers "what checks a change, and what does it produce?":

```mermaid
flowchart LR
    trigger["Dependabot PR or<br/>manual dispatch"] --> build["build job<br/>Linux, macOS, Windows"]
    trigger --> stable["stable job<br/>cargo check"]
    any["push to main, PR,<br/>weekly, dispatch"] --> audit["audit job<br/>RustSec"]
    build -->|"fmt, clippy, test"| profiles["Feature profiles<br/>all, core, headless"]
    profiles -->|"cargo build --release"| binary[Release binary]
    binary -->|"package"| zip["Portable ZIP<br/>smoke test"]
    classDef focus fill:#fde68a,stroke:#b45309,color:#1f2937
    class build focus
```

The `build` job on Rust 1.91 is the real gate: on each OS it runs rustfmt, Clippy
with `-D warnings` and the test suite for the all-features and core-only
profiles, builds release binaries, checks each optional feature on its own, and
finally packages the core binary and runs the extracted `run.sh` / `run.bat
--version`. The `stable` job only checks that the code still compiles on the
latest stable Rust. To save CI minutes, both run only for Dependabot pull
requests or a manual run; the RustSec dependency audit runs on every push to
`main`, every pull request and weekly. Nothing is uploaded or deployed: the ZIP
is built and discarded.

Where in the code:

- [.github/workflows/ci.yml](../.github/workflows/ci.yml)
- [.github/dependabot.yml](../.github/dependabot.yml)
- [rust-toolchain.toml](../rust-toolchain.toml) (pinned toolchain), [Cargo.toml](../Cargo.toml) (`gui`, `ocrs`, `onnx` features)
- Packaging: [src/infrastructure/distribution/](../src/infrastructure/distribution/)

## Public library surface and placement

The top-level modules in `src/lib.rs` (`core`, `device`, `scan`, `batch`,
`process`, `export`, `imaging`, `pipeline`, `config`, `cli`, `gui`, the
backend-named modules, and the rest) are the documented public library. Each is a
`pub use` list only, and `docs/release-notes.md` records changes to names and
signatures. The layer modules are private, so implementation types never leak.

Put new pure values and rules in domain, concrete I/O and platform behavior in
infrastructure, multi-step use cases in workflows, and CLI/GUI/plugin translation
in inbound. Expose something publicly only by adding it to a facade.

The architecture requires the dependency direction, the inbound infrastructure
allowlist, re-export-only facades, private layer modules, and no `#[path]` module
wiring.

## Image and export resource ownership

The public pipeline borrows its input and clones it once at entry. Workflows move
their owned decoded image into the private pipeline, and disabled stages return
that allocation. Scanner-profile JSON stays a public interchange format, while
export preparation builds a typed matrix/gamma transform once. PDF and TIFF keep
their public borrowed transform callbacks and use private owned loading paths for
production exports.

An export owns its OCR job. OCRS checks the active pointer and manifest and hashes
each model on every page, then reuses the engine only while the verified model
content is unchanged. A corrupt or invalid replacement returns an error; the job
never falls back to the previously selected engine. Chunked reads and the
boundaries around construction and inference check cancellation, but an active
OCRS inference call cannot be interrupted internally.

When a batch requests several aggregate outputs, a private publication session
shares one page loader. It caches decoded published pages as packed pixels in a
private temporary directory, capped at 512 MiB including cache headers. Each reuse
rechecks the source content. Cache failures and exhausted capacity fall back to
ordinary decoding, and source and encoder errors stay visible. TIFF/PDF/sheet
publication stays sequential, so a later failure preserves earlier published
outputs. RAII cleanup removes the private cache on normal return, error,
cancellation, and unwinding. PDF still retains compressed streams until its atomic
publication, and decoded source pages are processed one at a time.

GUI startup, refresh, and selected-device capability probes share one background
worker with a coalesced pending request and generation checks. Rendering reads
cached preview statistics and textures. An image change invalidates both caches,
and maintenance stays disabled until current capabilities arrive. Closing the
window cancels discovery and defers exit until its worker has finished.

ONNX fills its final target-sized tensor from packed input pixels, without a
source-sized float expansion. Worker identity verification, the private execution
image, process isolation, and resource limits form the inference boundary. The
`gui`, `ocrs`, and `onnx` Cargo features select implementations, while
public facades and CLI parsing stay present in every profile.

## Code map

Read in this order to follow a scan from the command line to the file on disk.
The [README](../README.md#repository-structure) lists the layers; this table
names the files inside them.

| Path | Responsibility |
| --- | --- |
| [src/main.rs](../src/main.rs) | Executable entry; calls `cli::run_from_env` |
| [src/inbound/cli/args.rs](../src/inbound/cli/args.rs) | clap definitions for every subcommand |
| [src/inbound/cli/handlers/router.rs](../src/inbound/cli/handlers/router.rs) | Dispatches each subcommand to its handler |
| [src/inbound/cli/handlers/scan.rs](../src/inbound/cli/handlers/scan.rs) | Merges flags and config into `ScanToFileArgs` |
| [src/workflows/capture/single.rs](../src/workflows/capture/single.rs) | Single-scan use case: validate, open, acquire, publish |
| [src/workflows/capture/batch.rs](../src/workflows/capture/batch.rs) | Batch use case: page stream, naming, multipage outputs |
| [src/workflows/process.rs](../src/workflows/process.rs) | Shared page kernel: pipeline, profile, OCR, save |
| [src/workflows/publication.rs](../src/workflows/publication.rs) | Export validation, OCR dispatch, PDF or image choice |
| [src/infrastructure/acquisition/contract.rs](../src/infrastructure/acquisition/contract.rs) | `DeviceSession` trait every backend implements |
| [src/infrastructure/acquisition/registry.rs](../src/infrastructure/acquisition/registry.rs) | Device id → backend session |
| `src/infrastructure/acquisition/{mock,file,sane,wia,escl}.rs` | The five backends |
| [src/domain/processing/apply.rs](../src/domain/processing/apply.rs) | `apply_pipeline`: the ordered image operations |
| [src/infrastructure/media/](../src/infrastructure/media/) | Codecs, PDF, TIFF, OCR engines, ICC, page cache |
| [src/infrastructure/runtime/](../src/infrastructure/runtime/) | Child processes, temp files, atomic publication |
| [src/infrastructure/config/json.rs](../src/infrastructure/config/json.rs) | Load and save `config.json` |
| [src/inbound/gui/app/](../src/inbound/gui/app/) | GUI state machine, job threads, discovery |
| [src/infrastructure/onnx/](../src/infrastructure/onnx/) | ONNX worker launch, identity check, limits |
| [src/architecture_rules.rs](../src/architecture_rules.rs) | Test that enforces the layer direction |
