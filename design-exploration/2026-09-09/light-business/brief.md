Open Scanline: product discovery and light business UI exploration
=================================================================

Design exploration only. The recommended direction is light, high-key, restrained business software. The PNG files are generated mockups, not implemented screens or evidence of current GUI behavior.

Read-only discovery covered repository instructions, acquisition and processing domain values, workflow orchestration, backend and media adapters, functional contracts, and capability sections of documentation. Existing GUI source, styling, navigation, screenshots and visual snapshots were not inspected. The application was not launched. Existing worktree changes were preserved. Only design artifacts were added.

**Product and users**

Open Scanline acquires images from scanners or local files, processes them locally, and exports image files and documents. It also provides CLI and public Rust library entry points. This purpose is explicit in [README.md](../../../README.md) and [AGENTS.md](../../../AGENTS.md).

The selected primary user is an office administrator or independent professional digitizing paper records on a desktop or laptop beside a scanner. Their assumed goal is a legible, findable local document with a clear save location. Basic office-software familiarity is assumed; knowledge of eSCL, OCR engines, logical sides and scanner negotiation is not. This persona and the relative importance of document scanning are design assumptions, not verified user research.

Photo and film digitizers are credible secondary users because the domain includes film conversion, color correction and restoration. Technical operators and integrators are another secondary audience because CLI, configuration and library contracts exist. The primary scan job should expose their relevant controls through secondary disclosures without making every new document a calibration exercise.

| Verified task | Inputs and actions | Result and evidence |
| --- | --- | --- |
| Acquire an image | Select device, mode, resolution, dimensions and optional region; scan or preview | One processed image, optionally a separate pre-pipeline archive. `src/workflows/capture/single.rs:19–38, 101–119, 265` |
| Capture a document | Select feeder, simplex/duplex, side limit, page format and aggregate destination | Numbered page files plus optional multipage PDF/TIFF and contact sheet. `src/workflows/capture/batch.rs:25–53, 78–110, 323–353` |
| Process an existing image | Load a file, apply processing preferences, choose destination | Processed output, with optional profile correction and OCR before publication. `src/workflows/process.rs:14–85` |
| Correct images and film | Set crop, rotation, deskew, tone/color, cleanup, restoration or film parameters | Deterministic image transformations. `src/domain/processing/plan.rs:9–42`; `src/domain/processing/apply.rs:22` |
| Create searchable or protected PDFs | Choose PDF destination, OCR engine/language and optional runtime password | Searchable text layer and/or encrypted PDF. `src/workflows/publication.rs:110–156`; `tests/pdf_behavior.rs:17–79` |
| Use advanced local tools | Supply OCR models, scanner profiles or a trusted ONNX worker/model | Local OCR, profile correction or inference report. `docs/usage.md:68–153`; inference does not modify the input image |
| Configure or maintain a scanner | Save JSON preferences; request only supported maintenance | Durable settings and capability-dependent maintenance. `src/workflows/settings.rs:15–73`; `src/workflows/ports/acquisition.rs:58–145` |

Source paths above are relative to the repository root. Important readable references: [batch workflow](../../../src/workflows/capture/batch.rs), [processing plan](../../../src/domain/processing/plan.rs), [PDF contracts](../../../tests/pdf_behavior.rs), [workflow contracts](../../../tests/workflow_contract.rs), [capability documentation](../../../docs/backends.md).

**Constraints that shape the interaction**

- Mock and file sources need no hardware. File batches repeat an image; they are not multipage imports. WIA needs Windows/PowerShell and a compatible scanner; SANE needs `scanimage`; eSCL needs reachable network hardware. TWAIN is separate host integration. See `docs/backends.md:10–16, 62–64`.
- A batch limit is 1–1,000 logical image sides. Duplex requires ADF/document mode and an even limit. Six sides represent three double-sided sheets. A single-image scan rejects duplex. See `docs/usage.md:30, 54`.
- Early feeder exhaustion after complete pages/pairs can succeed with fewer pages. An empty feeder or incomplete duplex pair fails. Page files already published can survive a later failure; PDF assembly follows successful acquisition. See `src/workflows/capture/batch.rs:78–110, 376–405`.
- Device-supported sources and resolutions vary. A requested resolution can be negotiated to another supported value. The design must distinguish requested and actual settings when they differ. See `docs/backends.md:31, 50, 60`.
- Built-in offline OCR is a 5x7 template recognizer, not a general document-recognition guarantee. Tesseract requires its executable and language data. OCRS requires supplied local models, is an early preview for printed Latin English, and exposes no recognition confidence. There is no silent fallback when the selected engine fails. See `docs/usage.md:105–139`.
- Searchable text and password controls require PDF output. Passwords are runtime-only and must not be persisted in preferences. See `docs/usage.md:83–103`.
- Device discovery and direct network access have explicit trust constraints. A missing scanner should not silently enable arbitrary endpoint access. See `docs/backends.md:54–58`.

**Independent brief**

The primary task is to scan up to three double-sided English invoice sheets at a chosen 300 dpi into `September-invoices.pdf`, keeping the six PNG page files in a clearly named folder. This fictional example assumes a suitable eSCL scanner and installed Tesseract English support. It does not claim either is available on this machine. The 300 dpi choice is an example, not a repository default.

The primary context is a resizable desktop window operated with keyboard and pointer. Paper loading happens outside the application. The information hierarchy is job name, acquisition settings, output settings, destination and action, then live status and final files. The task needs no dashboard, persistent sidebar or historical queue.

Use ordinary sentence-case controls, near-white surfaces, restrained sans-serif typography, fine dividers and a muted blue action color. Keep inputs compact and related choices aligned. This supports office tasks without visual distraction or requiring users to interpret decorative metaphors. Keep file names and destinations stable as the job changes state.

**Two generated concepts**

| Concept | Interaction and audience fit | Tradeoff |
| --- | --- | --- |
| [Single workspace](01-prepare.png), recommended | Scan settings and output choices are visible together. Suitable for an operator who needs to check source, sides and destination before each job. | More choices visible initially; corrections and passwords need progressive disclosure. |
| [Focused sequence](alternative-focused-output.png) | Source selection precedes a narrow output sheet. The selected scanner remains summarized and editable through Change. Suitable for occasional users who benefit from fewer simultaneous choices. | More navigation; source/output relationships are less immediately visible. |

Both use the same verified capabilities and the requested light business style. The workspace is recommended because this job has a small enough set of essential settings to review together.

**Recommended sequence**

1. [Prepare](01-prepare.png): select scanner, feeder, both sides, six-side limit and resolution; name the PDF and page folder; select searchable text and engine/language; choose a destination; start scanning. “Use an image file” branches to the verified single-image processing use case.
2. [Scanning](02-scanning.png): keep the job title and destination fixed. Replace settings with progress and a read-only summary. Distinguish saved PNG files from the PDF that has not yet been created. The bar measures captured/published sides against the limit, not total processing time.
3. [Saved](03-saved.png): report the actual page count, PDF and image outputs, destination and reason scanning stopped. The example reaches its six-side limit, so the user should check whether more paper remains. New scan begins another job; Done dismisses the result.

The generated completion image inherited a pencil beside the job title. That glyph is a generation artifact: post-save file renaming is not part of this proposal. Remove it in any later prototype. The preparation title is merely an editable output-name convenience, not a persistent job-management feature.

**Proposed interface capabilities and implementation dependencies**

The layouts, navigation and responsive behavior are proposed. They do not establish which features the current GUI exposes; that remains deliberately unexamined.

The live per-file saved manifest, precise waiting/OCR phase labels, affirmative tool-availability status and surfaced terminal reason are proposed interface capabilities. The underlying acquisition, OCR and publication operations exist, but these displays require reliable bindings and, where needed, additional typed workflow events. In particular, `BatchPageCollector::report_progress` runs before processing/publication, not after it (`src/workflows/capture/batch.rs:323–362`). It cannot by itself prove that the current page was saved. The acquisition end reason is validated but is not retained in the returned `BatchPages` structure (`batch.rs:366–405`). Preserve and expose that reason before making an exact completion claim.

Tool readiness must be checked, not inferred from a configured engine name. The mockups label the available tool as an example. The design adds no automatic OCR fallback, classification, accuracy score, page reordering, resumable capture or cloud service.

**Responsive and accessible behavior**

At wide desktop sizes, retain two columns within a bounded content width. At narrower widths, stack scan settings before output, followed by destination and action. At high text zoom, use the same single-column order and let filenames wrap. Keep the action area in normal flow when a sticky footer would obscure controls. The focused alternative already follows this narrow reading order.

A phone camera workflow, browser scanner access and mobile app are outside the verified product scope. Narrow-window behavior is a desktop adaptation, not a mobile acquisition claim.

Use persistent programmatic labels, sensible keyboard order, visible focus, keyboard-operable disclosures and explicit units. Expand “Both sides” to “Both sides, duplex” in accessible text; associate the sheet-count explanation with the side limit. Status changes need polite announcements, with errors announced once and focus moved to the relevant recovery action. Do not announce every progress tick. Distinguish states by text and icon, not color alone. Target comfortably sized controls, text enlargement without clipping and contrast suitable for ordinary office lighting. The image palette and interaction accessibility are not measured or tested; verify them in an implemented prototype.

**States beyond the generated sequence**

| State | Proposed behavior and plain-language copy |
| --- | --- |
| Discovering devices | “Looking for scanners…”; do not briefly report an empty list while discovery is pending. |
| No scanner | “No scanner found.” Offer refresh and the existing image-file branch. Explain unavailable backends locally; do not invent a device. |
| Unsupported duplex/source | Explain the selected scanner’s limitation beside the relevant control. Offer supported settings without changing them silently. |
| Invalid limit | “Both sides requires an even number of image sides.” Keep the value editable; identify the 1–1,000 side bound. |
| OCR unavailable | Explain the missing engine/language/model requirement. Let the user explicitly select an available engine or turn off searchable text. Never substitute the template engine silently. |
| PDF assembly/OCR loading | Keep capture count and current phase separate. Use indeterminate phase progress when a trustworthy duration is unavailable. |
| Empty feeder | “The document feeder is empty. No pages were saved.” Only use the second sentence after confirming outputs. |
| Incomplete duplex or acquisition error | Report the error and actual published files: for example, “Scanning stopped after 3 sides. The PDF was not created.” Do not offer Resume without new backend support. |
| Cancel requested | “Cancelling…” until the operation has stopped. Then list confirmed files retained and any outputs not created. |
| Output write failure | Identify the failed destination and retain the visible inventory of successfully published outputs. A later retry must have explicit duplicate/overwrite handling. |
| Successful early exhaustion | Report actual count and “The feeder is empty.” Do not imply the configured maximum was reached. |
| Successful limit reached | Report actual files and “Stopped at the 6-side limit.” Avoid claiming the entire physical stack is complete. |

**Assumptions, gaps and validation**

Audience frequency, predominant document types, target operating systems, language needs and physical scanning practices need user research. No personas, actual scanners, installed tools, live performance or scanning success were inferred from this workstation. The chosen invoice filenames contain no actual captured content.

Essential scan dimensions/region remain a handoff dependency: workflow arguments include width and height, while hardware adapters negotiate physical regions differently. Before implementation, define a backend-grounded full-bed/default-region policy or add a visible acquisition-area control. Do not silently label the example A4 or promise automatic paper-size detection.

No UI code was read to resolve presentation-only gaps. No implementation, Rust tests, hardware tests or desktop runtime checks were performed for this design-only task. Source and functional tests were read as capability evidence. Generated images were visually inspected for content, legibility and sequence consistency. They illustrate hierarchy and appearance, not functioning controls, measured accessibility, animation or tested usability.

A later prototype should test whether operators distinguish sides from sheets, understand that PNG pages are also saved, locate their output, recover from unavailable OCR, and correctly interpret early feeder exhaustion and partial failure. Validate keyboard use, 200% text scaling, narrow windows, real backend negotiation and terminal-state reporting.

All final images and the exact built-in image-generation prompt set are in this folder. [prompts.json](prompts.json) records the prompts; earlier images outside this folder are superseded explorations.
