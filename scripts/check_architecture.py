"""Enforce private implementation layers and compatibility facades."""

import pathlib
import re
import sys

from rust_tokens import path_segments, tokens, use_paths

FACADES = set(
    "batch cli config core device escl export features film gui i18n icc "
    "imaging manufacturers ml ocr packaging pipeline platform plugin process "
    "sane scan twain wia".split()
)
LAYERS = set("domain error inbound infrastructure operation workflows".split())

# Inbound may reach `crate::infrastructure::<...>` only through these
# `(module, name)` prefixes. Anything that opens an acquisition session and
# acts on it, or assembles output through more than one media adapter, is a
# workflow use case and must be called through `crate::workflows::*`
# instead, even though `infrastructure` itself is not a forbidden root for
# `src/inbound/`. See docs/architecture.md and the task handoff for the
# one-line justification behind each entry.
INBOUND_INFRASTRUCTURE_ALLOWLIST = {
    # Device inventory/capability snapshots for the GUI device picker and
    # CLI `devices`/`info` output. Opening a session to scan, batch, or run
    # a maintenance action stays behind workflows::capture / ::maintenance.
    ("acquisition", "list_all_devices"),
    ("acquisition", "list_all_devices_with_cancellation"),
    ("acquisition", "list_backends"),
    ("acquisition", "list_backends_with_cancellation"),
    ("acquisition", "find_scanners_with_cancellation"),
    ("acquisition", "maintenance_capabilities_with_cancellation"),
    ("acquisition", "DeviceInfo"),
    ("acquisition", "DeviceMaintenanceCapabilities"),
    ("acquisition", "ScanPagesEnd"),
    ("acquisition", "sane"),
    ("acquisition", "wia"),
    # Config JSON persistence and the AppConfig value type: CLI `config`
    # subcommands and GUI settings load/save.
    ("config", "json"),
    ("config", "AppConfig"),
    # Read-only runtime platform diagnostics, headless availability probes,
    # and the RAII temporary-file value type GUI jobs stage working images
    # in; none of these compose more than one adapter.
    ("runtime", "platform"),
    ("runtime", "availability_probe_succeeds"),
    ("runtime", "CommandSpec"),
    ("runtime", "TemporaryOutput"),
    # Distribution packaging for `open-scanline package`.
    ("distribution", "build_portable"),
    ("distribution", "PackagingOptions"),
    # Single-adapter media convert/codec helpers for CLI `convert` and GUI
    # open/preview. Multi-step output (multipage/PDF assembly, ICC-aware
    # publication, scanner profiling) stays in workflows.
    ("media", "convert_image_with_cancellation"),
    ("media", "load_image"),
    ("media", "image_buffer_to_rgba"),
    ("media", "save_image"),
    ("media", "supported_extensions"),
    ("media", "ocr"),
    # ONNX inference CLI entry points, including the hidden `__onnx-worker`
    # subprocess re-entry point.
    ("onnx", "run_isolated_onnx_with_executable"),
    ("onnx", "run_onnx_worker"),
    ("onnx", "OnnxInferenceOptions"),
    ("onnx", "OnnxInputLayout"),
    ("onnx", "OnnxNormalization"),
}


def forbidden_roots(path):
    if path.startswith("src/domain/"):
        return FACADES | {"infrastructure", "inbound", "workflows"}
    if path.startswith("src/workflows/"):
        return FACADES | {"inbound"}
    if path.startswith("src/infrastructure/"):
        return FACADES | {"workflows", "inbound"}
    if path.startswith("src/inbound/"):
        return FACADES
    return set()


def inbound_infrastructure_violation(path, segments):
    """`segments` is `("infrastructure", ...)`; check its allowlist prefix."""
    if not path.startswith("src/inbound/"):
        return False
    return tuple(segments[1:3]) not in INBOUND_INFRASTRUCTURE_ALLOWLIST


def expression_dependency(items, index):
    segments = path_segments(items, index)
    if segments:
        yield segments[0], items[index][1], tuple(segments)


def import_items(items, start):
    for token, _ in items[start:]:
        if token == ";":
            return
        yield token


def import_dependencies(items, start, offset):
    for path in use_paths(iter(import_items(items, start))):
        if len(path) > 1 and path[0] == "crate":
            yield path[1], offset, tuple(path[1:])


def crate_aliases(items):
    text = " ".join(token for token, _ in items)
    direct = re.findall(r"\buse crate as (\w+) ;", text)
    grouped = re.findall(r"\buse crate :: \{ self as (\w+)", text)
    return {"crate", *direct, *grouped}


def dependencies(items):
    """Include expression paths as well as grouped, multiline use trees."""
    roots = crate_aliases(items)
    normalized = [("crate" if word in roots else word, pos) for word, pos in items]
    previous = None
    for index, (token, offset) in enumerate(items):
        # A root token that heads a `use` path is fully handled by
        # `import_dependencies` below; also walking it as a bare expression
        # path would naively swallow a following group's `{` as a fake path
        # segment (e.g. `use crate::infrastructure::acquisition::{...}`).
        if token in roots and previous != "use":
            yield from expression_dependency(items, index)
        elif token == "use":
            yield from import_dependencies(normalized, index + 1, offset)
        previous = token


def is_facade_path(path):
    posix = pathlib.PurePosixPath(path)
    return posix.stem in FACADES and path.count("/") == 1


def structural_errors(path, text):
    if re.search(r"#\s*\[\s*path\s*=", text):
        yield "path-module wiring is not allowed"
    if path == "src/lib.rs":
        for name in re.findall(r"\bpub\s+mod\s+(\w+)\s*;", text):
            if name in LAYERS:
                yield "implementation layers must remain private"
    if is_facade_path(path):
        declaration = r"\b(?:fn|struct|enum|trait|impl|const|static|type|mod)\s+\w+"
        if re.search(declaration, text):
            yield "compatibility facade contains implementation"


def violations(path, source):
    items = list(tokens(source))
    forbidden = forbidden_roots(path)
    seen = set()
    for root, offset, segments in dependencies(items):
        if root == "infrastructure" and inbound_infrastructure_violation(path, segments):
            if (root, offset) in seen:
                continue
            seen.add((root, offset))
            line = source.count("\n", 0, offset) + 1
            dotted = "::".join(segments)
            yield f"{path}:{line}: inbound import outside the infrastructure allowlist: crate::{dotted}"
            continue
        if root not in forbidden:
            continue
        if (root, offset) in seen:
            continue
        seen.add((root, offset))
        line = source.count("\n", 0, offset) + 1
        yield f"{path}:{line}: forbidden dependency crate::{root}"
    normalized = " ".join(token for token, _ in items)
    for error in structural_errors(path, normalized):
        yield f"{path}: {error}"


def main():
    errors = []
    for path in sorted(pathlib.Path("src").rglob("*.rs")):
        errors.extend(violations(path.as_posix(), path.read_text()))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("architecture check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
