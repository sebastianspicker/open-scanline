# Light business workspace implementation review (2026-09-09)

At this review point, the native egui application followed the approved [preparation](../01-prepare.png), [scanning](../02-scanning.png) and [completion](../03-saved.png) concepts. This is historical QA evidence for the exploration; the later Platen direction is recorded in [the repository design brief](../../../../DESIGN_BRIEF.md). Controls were real widgets connected to existing acquisition, processing and publication behavior. No generated image was used as a substitute for a working screen.

## Scope and behavior

The primary scan workspace uses a bounded two-column layout, near-white surfaces, restrained blue actions, bundled Cantarell typography and fine dividers. Narrow windows stack acquisition before output. Short windows scroll in normal reading order, including the destination and primary action, as specified in the design brief.

Source, capture mode, duplex, resolution, side limit, acquisition dimensions, output format, OCR, password and destination remain editable through real settings. Image corrections lead to the retained image tools. Advanced actions preserve the existing menus, configuration recovery, profiles and specialized operations.

Running jobs lock settings and expose cancellation. Job reports snapshot requested settings and distinguish published files from requested outputs. Pages and raw images are reported only after successful publication; failures and cancellation retain confirmed outputs. Successful batches distinguish the side limit from feeder exhaustion. Existing files on disk are not treated as evidence of success in the current job.

Relevant implementation:

- `src/inbound/gui/view/theme.rs`: shared appearance and system-dark-mode regression coverage.
- `src/inbound/gui/view/workspace.rs` and `workspace/`: preparation, validation, responsive composition, progress and results.
- `src/inbound/gui/app/report.rs` and app lifecycle modules: job snapshots and terminal state.
- `src/workflows/capture/batch.rs` and `batch/outputs.rs`: post-publication events and aggregate output publication.
- `src/inbound/gui/actions/helpers.rs` and `src/inbound/gui/state.rs`: structured error feedback.

## Native screenshots

Local JPEG captures of the running macOS application were reviewed using an isolated configuration and generated mock input. They covered the wide preparation, scanning, saved, narrow-window and invalid-filename states. The captures are intentionally not tracked because their UI includes workstation-specific output paths; they contain no captured personal documents and are not used as substitutes for the working interface.

Wide captures were 1343 × 768 pixels and narrow captures were 615 × 841 pixels. Native resizing also checked the minimum-width layout.

## Verification

All eight required repository checks passed: architecture, formatting, Clippy in both feature profiles with warnings denied, tests in both profiles, and both release builds. Final totals were 158 passing tests with all features and 97 passing tests without defaults. Three and one existing release-only benchmarks, respectively, remained ignored. `git diff --check` also passed. Commands and exit codes are recorded in [checks.json](checks.json); detailed local logs are under `target/gui-review/checks/`.

Native interactions verified:

- Six-side mock acquisition with searchable PDF enabled completed and listed six PNGs plus the PDF. Independent `pdfinfo` inspection confirmed six pages.
- A 100-side batch was cancelled after 24 completed pages. Those 24 PNGs remained on disk; no aggregate PDF was created for that cancelled run.
- An invalid filename disabled Start scan and showed an inline reason. Correcting it restored the action. Keyboard text editing and Tab movement to the next control were verified.
- Loading an actual saved mock page through the native file dialog opened the image tools. Re-process completed and published a real PDF.
- Wide and narrow layouts, long-text wrapping, system-dark-mode isolation and disabled controls were checked through native rendering and focused egui tests.

Calculated palette contrast ratios are 15.89:1 for body text, 5.64:1 for muted text, 6.04:1 for white primary-button text and 6.25:1 for error text. These calculations do not replace screen-reader or full accessibility testing.

## Intentional departures and limits

Existing output naming is preserved: batch pages use `batch/page_001.png` and the document uses `<name>_multipage.pdf`. The workspace shows these actual paths rather than adding independently editable document/page-folder names from the illustration. The title follows the existing filename base. Requested pixel dimensions are exposed instead of promising automatic paper-size detection.

The screenshots truthfully identify the mock source and built-in template recognizer. They do not imply that an office scanner or Tesseract is installed. The specialized image editor keeps its existing organization within the shared light theme. Additional capture/output controls and advanced actions preserve capabilities absent from the mockup.

Physical scanners, WIA/SANE/eSCL feeder behavior, Windows/Linux desktop rendering, external OCR executables and real-document OCR quality were not exercised. New workspace copy is English; existing language preferences and translated editor features remain available. Full screen-reader announcements, 200% text enlargement and task usability with representative operators still need validation.

Review configuration and generated output remain under ignored `target/gui-review/`. Personal configuration was not used. The local native captures are excluded as described above.
