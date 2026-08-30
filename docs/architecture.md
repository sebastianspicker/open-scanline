# Architecture

Open Scanline is a native Rust application. Its library serves the command-line
program, the optional desktop GUI, and the headless plugin and host entry
points. Existing top-level modules remain as compatibility facades; the
implementation lives in the layers below.

## Layers

`src/domain/` contains the typed scan request, image buffer and geometry,
processing preferences, calibration, and pure processing operations. It may
depend on its own modules and shared error values, never on adapters, entry
points, or compatibility modules.

`src/workflows/` implements the capture, batch, processing, publication, and
settings use cases. Its narrow `AcquisitionPort` and `MediaPort` describe only
the capabilities capture and publication workflows need. A workflow expresses
the operation and its cancellation, validation, ordering, and publication
rules; it does not choose a scanner, codec, configuration format, or UI.

`src/infrastructure/` supplies those concrete capabilities: mock/file/SANE/WIA
and eSCL acquisition, JSON settings, media codecs/PDF/OCR/ICC, ONNX worker
isolation, process supervision, atomic publication, and portable archives.

`src/inbound/` adapts external requests into workflows. It owns CLI parsing and
output, the optional GUI, diagnostics, plugin mode, and host integration.
`src/composition.rs` is the native composition root: it wires
`NativeAcquisition` and `NativeMedia` for workflow entry points. Device catalog
and maintenance operations, and JSON settings persistence, are direct inbound
adapter concerns rather than workflow ports.

The dependency direction is:

```text
inbound -> composition -> workflows <- infrastructure
                         |
                         v
                       domain
```

Infrastructure implements workflow ports. Domain has no outward dependency;
workflows do not import infrastructure or inbound modules.

## State and data flow

An inbound adapter constructs typed input and invokes a workflow with the
needed port. Capture opens one device session, produces one image or a bounded
stream of logical sides, applies domain processing, then publishes through the
media port. Batch keeps page paths in order and assembles optional documents
after page publication, rather than retaining every decoded page. JSON settings
are loaded and saved by the inbound edge through the settings adapter;
configuration and language are application-instance state, while translation
catalogs are immutable data.

Scanner artifacts, OCR input, configuration, profiles, and package outputs are
validated before same-directory atomic publication. Cancellation is carried
through the operation and into command-backed adapters.

## External contracts

`DeviceSession` is the acquisition-session contract. Mock and file backends
are deterministic hardware-free sources; physical backends supply the same
contract. Batch limits are logical image sides, from 1 through 1,000. A
single-image duplex request is rejected rather than losing a side.

SANE needs `scanimage`; WIA needs Windows PowerShell/COM and compatible
hardware; eSCL uses bounded HTTP(S) discovery or an explicit endpoint. An
unlisted eSCL endpoint requires the strict `--allow-unlisted-escl` opt-in.
TWAIN support starts a separate host integration: it is not a native TWAIN Data
Source or an acquisition backend.

OCR and JPEG XL rely on explicitly selected optional executables. User ONNX
models run locally in a supervised worker with resource limits, not in a
filesystem or syscall sandbox. A working build or test suite cannot prove a
physical scanner, driver, feeder, firmware, desktop session, or optional tool
on a target machine.

## Compatibility and placement

The top-level `core`, `device`, `scan`, `batch`, `process`, `export`,
`imaging`, `config`, `cli`, `gui`, and backend-named modules preserve established
library paths. Keep those facades thin; `tests/public_api_contract.rs` protects
the names and signatures consumed outside the new layer tree. The domain,
workflow, infrastructure, inbound, and composition modules are private so
adapter implementations do not become accidental public API.

Put new pure value/processing logic in domain, use-case coordination and port
traits in workflows, concrete I/O and platform behavior in infrastructure, and
CLI/GUI/plugin translation in inbound. Add production wiring only in
composition. `scripts/check_architecture.sh` enforces these mechanical
boundaries and rejects path-module wiring.
