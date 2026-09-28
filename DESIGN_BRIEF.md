# Open Scanline design brief

Status: implemented on branch `redesign/platen` (2026-09-28). This brief covers
the desktop GUI (`src/inbound/gui/`) and the GitHub Pages tour (`docs/index.html`).
The CLI, library, configuration, and file formats are out of scope and unchanged.

## 1. Product

Open Scanline takes pages from a scanner (or an image file) to finished files on
the local disk: page images, a multipage PDF or TIFF, and optionally a searchable
or password-protected PDF. It ships as a CLI, an egui desktop app, and a Rust
library. The README states the product's central promise: *no cloud service, no
accounts, no telemetry.*

**Primary journey (GUI).** *Prepare → Scanning → Saved.*

1. **Prepare**: choose scanner, paper source, single image or multiple sides,
   one side or duplex, resolution, side limit; name the output, pick a format,
   optionally make the PDF searchable (engine and language) or set a runtime-only
   password; choose a folder; start.
2. **Scanning**: pages are published one at a time. The document (PDF/TIFF) is
   assembled only after capture succeeds. The user can cancel.
3. **Saved / stopped / cancelled**: the report lists exactly which files exist,
   which requested files were not created, and why capture stopped (side limit
   vs. empty feeder).

**Moment of value.** The Saved screen: the job's name, a list of real files on
disk, and a clear statement of why scanning stopped. Everything before it is
setup; everything after it happens in the user's file manager.

**Secondary journey.** "Image tools": open an existing image, apply crop,
rotation, filters, color and film profiles, then save, OCR, or reprocess. It is a
legacy, dense, translated panel layout, reached from the header.

## 2. Audience

**Primary: the person standing next to the scanner.** Someone digitizing paper
records on their own machine: a freelancer or small-office administrator doing
invoices and contracts, or a privacy-minded Linux/macOS user archiving household
paperwork. They are comfortable with desktop software and file folders, and
usually aren't scanner experts: "eSCL", "ADF" and "side limit" are not their words.

- **Goals**: a correctly named, findable, legible file in a known folder, first time.
- **Anxieties**: *Did it get every page? Which folder did it go to? Did the back
  sides come out? Is my document being uploaded somewhere?*
- **What they distrust**: vague success messages, cloud nudges, marketing tone
  inside a tool, settings that change silently.
- **Daily tools**: a file manager, a PDF viewer, an office suite, a browser.
- **What signals quality to them**: exactness. File names shown as they will be
  written, counts that add up, honest statements about what did *not* happen,
  and controls that stay where they were.

**Secondary: photo and film digitizers** (film profiles, dust/grain controls,
color). They care about image fidelity and a neutral surround that doesn't tint
their judgement.

**Secondary: developers and integrators** reading the GitHub Pages site and
README. They judge the project on honesty about platform limits, clear commands,
and whether the docs look maintained. For the website, they are the primary
reader.

## 3. Brand character

| Trait | Not |
| --- | --- |
| **Exact**: counts, names and paths are shown literally | pedantic, or full of jargon |
| **Candid**: says what did not happen as clearly as what did | alarmist or apologetic |
| **Quiet**: the paper is the subject, the interface is the platen | bland or anonymous |
| **Physical**: sheets, sides, feeders, a light passing over glass | skeuomorphic or cute |
| **Self-contained**: local, private, no third parties | defensive or preachy about privacy |

## 4. Market observations

Competitors and alternatives: GNOME Document Scanner (Simple Scan), NAPS2,
VueScan, SilverFast, gscan2pdf, and scanner-vendor utilities. (From category
knowledge; no live sites were browsed.)

- **Conventions to honor**: settings are visible before scanning; page counts;
  a single obvious Scan action; file-format vocabulary (PDF, PNG, TIFF);
  platform-native file dialogs.
- **Conventions to break**:
  - *The toolbar of 20 identical icon buttons* (NAPS2, gscan2pdf, and our own
    Image tools). Here, one primary action per screen.
  - *Preview as the hero*: most scanning apps center an empty grey preview
    rectangle. Open Scanline's batch path has no pre-scan preview, so an empty
    preview area would be a lie. The hero here is **the output plan**: what files
    will exist.
  - *Pro-tool darkness* (SilverFast, VueScan dark skins). Documents are judged
    against paper, not against a black surround.
  - *Vague success* ("Scan complete!"). We list files and the stop reason.
- **Web**: open-source project pages default to centered hero, three icon
  cards, and a gradient. Ours reads like a job ticket and a spec sheet.

## 5. Current state

- **Stack**: egui 0.32 / eframe (native, glow), rfd file dialogs. Theme in
  `src/inbound/gui/view/theme.rs` (colors inline, no tokens). Workspace views in
  `view/workspace/{header,settings,status}.rs`; legacy tools in `view/panels.rs`
  and `view.rs`. Pages site: one static HTML file + one CSS file, no build step.
- **Brand assets**: no logo or icon; the wordmark is plain text. The only brand
  color is a muted steel blue (`#3A6788`). Cantarell is embedded as the PDF
  text-layer glyph carrier and was reused as the UI font. **Nothing carries
  brand equity worth preserving** except the name, which is the concept.
- **Worth keeping**: the Prepare / Scanning / Saved structure; the honest
  report model (`JobReport`: requested vs. published outputs, stop reason);
  validation that disables Start instead of failing later; the light workspace.
- **Weaknesses**:
  - Generic: steel-blue button, default egui combo boxes, Cantarell at one weight,
    every heading the same size. Nothing is specific to scanning.
  - Hierarchy: the Start action sits below the fold at common window heights
    (visible in the 1280×860 capture); "Advanced actions" is a bordered button
    that out-shouts the primary nav.
  - The file plan is hidden in small grey notes ("Document: batch/…").
  - Scanning state lists the PDF as "Not saved by this scan" *while it is still
    scanning*, which is false and alarming.
  - "Done" and "New scan" do exactly the same thing (`clear_job_report`).
  - The Scanning/Saved "Save in" shows the page folder, not the save folder.
  - Spinner beside a progress bar: two progress idioms at once.
  - Image tools: a 25-button toolbar, raw keys like "↔", a heading that
    reads "open-scanline 1.0.0".
  - Website: a system-font version of every open-source page, with a two-button hero.

## 6. Constraints

- Preserve every GUI action and its dispatch (`start_scan`, `start_batch`,
  `cancel_job`, `clear_job_report`, save/OCR/reprocess, maintenance, config
  recovery), all validation (`can_start`), and the tests that pin them
  (`view/workspace/tests.rs`, `gui/tests.rs`, `app/tests.rs`).
- No new crates. Fonts must be open-licensed and recorded in
  `THIRD_PARTY_NOTICES.md`. Cantarell stays (PDF text layer).
- Architecture rules (`scripts/check_architecture.sh`) and the full gate in
  `AGENTS.md`. Lizard warns on long functions, so keep view functions small.
- i18n: the workspace copy is English-only in code today, and the tools panel is
  translated through `assets/i18n`. egui's default fonts stay as fallbacks so
  Cyrillic, Greek and other scripts keep rendering.
- egui has no OS "reduce motion" signal. Motion must be non-essential and
  minimal.
- Screenshots must show only the mock source, with no personal paths or documents.
- The website is static GitHub Pages: no build step, no trackers, and ideally
  no third-party requests at all (it's a no-telemetry product).

## 7. Assumptions log

| # | Assumption | Evidence | Confidence |
| --- | --- | --- | --- |
| A1 | Primary GUI user digitizes office/household documents; film is secondary | Workspace defaults to multi-side PDF; Prepare exposes OCR and PDF password but film only in tools; earlier exploration brief chose the same persona | Medium |
| A2 | Light-only UI is right; no dark mode | Theme test pins light even under dark system appearance ("approved light workspace"); documents are judged against paper | Medium |
| A3 | Users think in *sheets* and *sides*, and need the duplex arithmetic spelled out | Existing copy "double-sided sheets maximum"; duplex requires even limits | High |
| A4 | Website readers are mostly developers evaluating the project | Page content is CLI, platform tables, docs links; hosted on GitHub Pages | High |
| A5 | The earlier steel-blue "light business" implementation is not protected brand equity | User requested a ground-up redesign; no logo or colors in docs other than CSS variables | High |
| A6 | Most batches are small (≤ 24 sides); larger ones need a compact progress form | Default 6; limit up to 1,000 | Medium |
| A7 | A static bottom action bar is preferable to in-flow Start | Start is below the fold at 1280×860 in the current capture; a bottom panel reduces the scroll area rather than overlaying content | High |
| A8 | Atkinson Hyperlegible Next is available and licensed for embedding | OFL 1.1 in google/fonts; static instances generated with fontTools | High |

## 8. Design direction

Three directions were developed. They differ in concept and structure, not just
color.

### Direction 1: "Platen" (chosen)

**Concept.** The window is the scanner bed and the job is a sheet laid on it.
One thing moves in the whole product: *the scan line*, a thin vermilion light
that crosses the page being captured. It gives the product its name. Everything
else is still, exact and paper-colored. The organizing artifact is the
**output plan**: the files that will exist, shown literally before, during and
after the scan.

*Why it fits*: the primary user's anxiety is "did every page make it and where
did it go". Platen answers that with literal file names, sheet/side arithmetic,
and a sheet rail that fills as pages publish. It is exact, candid, quiet,
physical and self-contained.

- **Typography**: *Atkinson Hyperlegible Next* (Regular 400, Bold 700) with
  *Atkinson Hyperlegible Mono* (400) for file names, counts and values. The
  family was drawn by the Braille Institute to keep easily confused characters
  apart (I/l/1, O/0, rn/m), which is also what OCR has to do. That makes it the
  right voice for a tool that turns pages into text and names files `page_001`.
  Scale (egui pt): display 34 / 27 narrow Bold · lead 17 · heading 19 Bold ·
  body 16 · label 14 Bold · small 13.5 · mono 14.5 · stage readout mono 12.5 caps
  with +1.2 tracking.
- **Color**: *paper* `#FAFAF7` (workspace), *platen* `#EFEEE9` (header and
  action bar, the bed's frame), *field* `#FFFFFF`, *ink* `#17191B` (text and
  primary action), *graphite* `#565B60` (secondary text), *rule* `#DCDBD5`,
  *control edge* `#8C8B84`, and one accent, *lamp* `#C8391F` (vermilion). Lamp
  marks the scan line, keyboard focus, and problems that stop a scan. No
  green-for-success: success is stated in ink with a drawn check mark.
- **Layout**: bounded 1120 pt sheet, two columns ("1 · Scanner and pages",
  "2 · Files") that stack below 760 pt. A persistent platen-colored action bar
  at the bottom holds destination + Start, so the primary action never scrolls
  away. Density is moderate: 8-pt rhythm, 40-pt controls, 46-pt primary action.
- **Motion**: only the scan line, sweeping the current sheet slot while a job is
  running (about 1.6 s per pass), with no easing flourish. No fades, springs or
  spinners. Everything else changes state instantly.
- **Signature details**: (1) the **sheet rail**, √2-proportioned slots for each
  requested side, paired for duplex, filled as pages publish, with the lamp
  line on the current one; (2) the **file plan ledger**, monospaced file names
  aligned in a two-column ledger with a drawn check mark once saved and a plain
  statement for anything not created.
- **Stands apart by**: no preview-as-hero, no toolbar, no blue, no dark pro skin.
  The hero is a list of files.
- **Refuses**: gradients, shadows, rounded cards, icon fonts, emoji, spinners,
  success green, marketing copy inside the tool.

### Direction 2: "Register" (archival ledger)

**Concept.** Scanning as accessioning: every job is an entry in a records
register. Serif display (*Source Serif 4*) over a grotesk, archival-board
grey-green `#4E5B52` with manila `#EFE6D2`. Jobs are entries with accession
numbers, and the Saved screen reads like a catalogue card. Motion: none.
Signature: accession-number stamp and catalogue-card manifest.

*Strength*: very distinctive, suits archivists. *Weakness*: implies a
persistent register or history that the product doesn't have (no job history,
no catalogue), and the manila palette tints the judgement of scanned documents.
It also reads nostalgic, not precise.

### Direction 3: "Darkroom" (film-first)

**Concept.** Near-black workspace, safelight-red accent, film-strip progress,
large preview. Type: a condensed grotesk plus mono. Signature: frames advance
like a film strip.

*Strength*: gorgeous for film and photo digitizers. *Weakness*: optimizes for a
secondary audience, puts an empty preview at the center of a batch flow that has
no preview, and the dark surround misrepresents paper documents. It is also the
category's "pro tool" cliché.

### Choice

**Platen**. It grows directly out of the name and the physical act, serves the
primary user's actual anxiety (completeness and location), and makes the
honest report model (the product's best asset) into the visual hero. It
trades away Register's warmth and Darkroom's drama. The cost of being quiet is
that it must be precise everywhere, because there is no decoration to hide behind.

**Robustness to low-confidence assumptions.** If A1 is wrong and film users
dominate, the neutral paper and graphite surround still suits photo judgement,
and the rail/ledger work the same for frames. If A2 is wrong, the token set
(paper/platen/field/ink/graphite/rule/lamp) maps one-to-one to a dark palette
without touching layout code. If A6 is wrong, the rail switches to a compact
proportional bar above 24 sides.

### Website application

Same tokens and type, self-hosted as WOFF2 (no Google Fonts request: a
no-telemetry product shouldn't phone a third party to render its homepage).
Left-aligned "job ticket" opening with a plain statement and the real CLI
command, not a centered hero. The tour follows Prepare → Scanning → Saved as
numbered steps, with capabilities as a spec table rather than icon cards. One
lamp line crosses the hero screenshot once on load and does nothing under
`prefers-reduced-motion`.

## 9. Implementation notes

- **Tokens** live in `src/inbound/gui/view/theme.rs` (`color`, `size`,
  `space`, `motion`) and are mirrored as CSS custom properties in
  `docs/assets/css/style.css`.
- **Components** (`view/workspace/components.rs`): stage readout, title, lead,
  numbered section, label, note, inline problem, job banner, primary /
  secondary / quiet buttons, field, choice, segmented control (stacks when its
  labels don't fit), drawn check mark, `~` path display.
- **Action bar** is an `egui::Area` pinned to the bottom edge and drawn after
  the form, so keyboard focus moves header → settings → destination → Start.
  A bottom panel would have put it first in the tab order.
- **Focus**: egui shares one visual state for pressed and focused widgets. The
  theme gives that state a 2 pt deep-lamp ring, and frameless buttons get an
  explicit ring, because egui draws none for them.
- **Motion**: egui exposes no OS reduce-motion preference. The scan line is the
  only continuous animation, runs only while a job is active, and duplicates
  information already given in text. Everything else is instant
  (`animation_time = 0`). The website honours `prefers-reduced-motion`.
- **Image tools** was tidied (tokens, grouped compact toolbar, no duplicate
  menu bar, named controls), not redesigned. It is the next candidate.
- **Behaviour changes** (all presentation-level): "Done" and "New scan", which
  called the same function, became one "Start another scan"; the tools-mode
  menu bar, which duplicated the header's More menu, was removed; the pending
  document is no longer labelled "Not saved by this scan" while capture runs.
