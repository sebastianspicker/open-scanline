# Security

## Reporting a vulnerability

Use the repository's private vulnerability-reporting channel in its Security tab. Do not disclose exploit details, sensitive documents, device identifiers, local paths, or credentials in a public issue.

If private reporting is not enabled, contact a project maintainer through the repository profile and ask for a private reporting channel before sharing details. Include the application version, operating system, affected backend or optional tool, reproduction steps, impact, and any practical mitigation.

## Local trust boundaries

Open Scanline processes local image files and configuration, communicates with selected or discovered eSCL scanners on the local network, and can invoke optional local executables such as `scanimage`, `tesseract`, and `cjxl`.

Treat scanner drivers, network scanners, external tools, input files, and user-supplied ONNX models as trusted only to the degree supported by the operating environment. Review their origin and permissions before use. The TWAIN path starts Open Scanline's plugin mode for a separate host or bridge, so use trusted host software.

Configuration is stored as JSON in the platform-specific `open-scanline` configuration directory. Keep it and scanned output protected according to their contents.
