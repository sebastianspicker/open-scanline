# Architecture

Open Scanline is an application with one Rust library used by its command-line, desktop, and headless host entry points. The library is internal application code rather than a compatibility promise for third-party crates.

## Shared workflow

The CLI and GUI construct typed scan, processing, batch, export, ONNX, and packaging options. A single scan returns one in-memory image buffer. The processing pipeline applies geometry, color, cleanup, film, automatic orientation, and automatic crop operations before one final encode. A validated scanner profile can then correct the final pixels, and searchable PDF OCR runs on those same pixels.

Batch acquisition is a streaming device-session contract. One open hardware session or protocol job emits bounded logical image sides to the pipeline and page saver; decoded acquisition buffers are not accumulated. SANE uses one `scanimage --batch` process, WIA one PowerShell/COM connection, and eSCL one created scan job with repeated `NextDocument` requests. After acquisition, optional PDF, TIFF, and contact-sheet outputs consume the ordered page paths. PDF assembly releases each decoded source page before loading the next, while the document retains compressed streams.

Mock and file sources are built in and provide deterministic repeated-page simulation, not physical ADF semantics. WIA, SANE, and eSCL implement the same device-session interface and retain one native session or protocol job for feeder batches. WIA and SANE command execution is isolated behind adapters so argument construction, parsing, cancellation, timeouts, cleanup, and decoding can be tested without physical hardware. Scanner-generated artifacts and Tesseract inputs use atomically reserved private temporary directories with drop cleanup. eSCL handles explicit hosts, mDNS, and bounded local subnet candidates through a rustls-backed HTTP client with response-size and discovery-deadline limits.

External scanner commands run in a contained process tree: a dedicated Unix process group or a Windows kill-on-close Job Object. Cancellation, timeout, and inherited-pipe failures terminate that tree before the parent returns. A deliberately daemonized Unix descendant that creates a new session can escape process-group containment; scanner tools are therefore still trusted local executables.

## OCR, models, and documents

OCR has two explicitly selected engines. The standalone `ocr` command uses Tesseract unless `--offline` is supplied. Searchable PDF export defaults to the built-in offline recognizer and uses Tesseract only when explicitly selected. Missing executables and failed external runs are reported rather than silently changing engines.

User-supplied ONNX models run locally through `tract` in a supervised child process. The parent uses bounded private-file IPC, a versioned worker protocol, a wall-time deadline, and kill-and-reap supervision; the worker applies static graph/tensor limits, single-thread environment controls, and hard memory limits on Linux and Windows. On macOS the parent samples resident memory and kills workers above the same 2 GiB ceiling because ordinary child-process address-space ceilings are unavailable; a brief allocation spike can occur between samples. If that sampling cannot be performed, the parent kills a still-running worker and fails closed. This is process and resource containment, not a filesystem or syscall sandbox. The input adapter supports static image sizes, RGB or single-channel tensors, NCHW and NHWC layouts, an optional input name, and typed tensor summaries.

PDF output is built as a structured document with `lopdf`. Pages contain encoded image objects and can include Unicode-searchable text, metadata, and PDF 2.0 AES-256 password encryption. Image, PDF, TIFF, config, scanner-profile, and portable-package publishers use same-directory temporary files and validate where appropriate before atomic publication. JPEG XL encoding is delegated to an installed `cjxl` executable so it remains an explicit optional tool.

## State and platform boundaries

Configuration and selected language belong to an application instance. Translation catalogs are immutable data, so one caller cannot change another caller's language through process-global state.

Native open and save dialogs are part of the optional GUI feature. Hardware and external-tool availability is discovered at runtime and reported factually. TWAIN-related code launches the headless host integration path; it does not implement a native TWAIN Data Source or acquire through TWAIN itself.
