# Open Scanline 0.1.0 alpha: known issues

Thank you for testing. This is an early alpha. It is meant to run on macOS
with a Canon imageFORMULA P-215II, and with the built-in mock and file
sources, which need no hardware. Below is what we already know doesn't work, or
hasn't been tried yet, and what to do about it.

## Not yet tried on real hardware

- **Canon P-215II on macOS.** Open Scanline drives this scanner through SANE
  (Homebrew `sane-backends`). SANE lists the P-215II as "untested", and we have
  not yet run a full session with the physical scanner. The single-page scan,
  the duplex batch and the "feeder ran out of paper" handling have only been
  checked against a simulated scanner. If something goes wrong here, a report
  helps us more than anything else.
  - Before you start, run `scanimage -L` in Terminal. If it doesn't list a
    `canon_dr:` device, Open Scanline can't see the scanner either. Please tell
    us; this is the single most important thing we need to know.
  - Set the scanner's Auto Start (CaptureOnTouch Lite) switch to **off**. With
    it on, the scanner presents itself as a different device.
  - The device id contains the USB port (for example
    `sane:canon_dr:libusb:001:004`) and can change when you replug the
    scanner. Run `open-scanline devices` again and use the new id.
- **The desktop app (GUI) has not been tested when started from Finder or the
  Dock.** It should find a Homebrew `scanimage` even without your Terminal
  `PATH`. If it reports that SANE is unavailable while the command line works,
  please report it. As a workaround, start the app from Terminal with
  `open-scanline gui`.
- **Windows (WIA), network scanners (eSCL/AirScan) and Linux** build, but no
  real device has been tried in this round. Reports are welcome, but expect
  rough edges.

## Things that work differently than you might expect

- **Existing files are not replaced.** If the output file already exists, the
  command stops with `output exists: …; pass --overwrite to replace it`. Add
  `--overwrite`, or choose a new name. This also applies when you run the
  quick-start commands a second time. The desktop app still replaces files.
- **The default scan area is the scanner's full area.** Use `--paper a4` (or
  `a5`, `a6`, `letter`, `legal`) to limit it. Settings saved by older builds
  with a 320 × 240 scan area are reset to the full area automatically.
- **Duplex needs `--source adf`** and an even `--pages` count, because
  `--pages` counts sides, not sheets. Without `--source`, the P-215II scans the
  front side only.
- **Built-in text recognition is very limited.** The default `offline` OCR
  engine only reads large, clean capital letters, so searchable PDFs made with
  it usually contain no text. For real documents, install Tesseract
  (`brew install tesseract`) and add `--ocr-engine tesseract`. The `ocrs`
  engine is an early preview.
- **Slow scanners can hit timeouts.** A single scan gets 90 seconds and each
  batch side 45 seconds. Raise these with `OPEN_SCANLINE_SCAN_TIMEOUT_SECS`, for
  example `OPEN_SCANLINE_SCAN_TIMEOUT_SECS=300 open-scanline scan …`.
- **Network scanners with self-signed HTTPS certificates** are refused unless
  you pin the certificate. The usage guide explains how
  (`OPEN_SCANLINE_ESCL_CERT_PINS`). Plain HTTP scanners are unaffected.
- **If a batch fails half-way** (for example a paper jam), the pages scanned
  before the failure are kept, and the error message says where they were
  saved.

## How to report a bug

Open an issue at <https://github.com/sebastianspicker/open-scanline/issues>.
Security problems go through the private channel described in `SECURITY.md`,
not a public issue.

Please include:

1. The output of `open-scanline --version` and your macOS version
   (Apple menu → About This Mac).
2. The exact command you ran, re-run with `--verbose` before the subcommand,
   and its complete output. For example:
   `open-scanline --verbose scan --device sane:canon_dr:… --out page.png`
   For the desktop app, start it from Terminal with
   `OPEN_SCANLINE_DEBUG=1 open-scanline gui` and copy what Terminal prints.
3. The output of `open-scanline devices`.
4. For scanner problems: the output of `scanimage -L` and of
   `scanimage --help -d <your canon_dr id>`.
5. Your settings: `open-scanline config --show`.
6. What you expected, and what happened instead.

Please don't attach scans of personal documents. If the problem depends on the
image, scan a blank or printed test page. The `--verbose` output shows commands
and file paths, but never PDF passwords. Check it before posting and remove
anything you would rather not share, such as your user name in paths.
