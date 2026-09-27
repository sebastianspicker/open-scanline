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
LAYERS = set(
    "composition domain error inbound infrastructure operation workflows".split()
)


def forbidden_roots(path):
    if path.startswith("src/domain/"):
        return FACADES | {"infrastructure", "inbound", "workflows", "composition"}
    if path.startswith("src/workflows/"):
        return FACADES | {"infrastructure", "inbound", "composition"}
    if path.startswith("src/infrastructure/"):
        return FACADES | {"workflows", "inbound", "composition"}
    if path.startswith("src/inbound/"):
        return FACADES
    return FACADES if path == "src/composition.rs" else set()


# Temporary, documented carve-outs for ports still mid-migration. Each one
# narrows a forbidden root back down to the specific path segments the
# in-progress dependency-inversion slices still rely on.
def is_exempt(path, segments):
    if not segments:
        return False
    if path.startswith("src/infrastructure/") and tuple(segments[:3]) == (
        "workflows",
        "ports",
        "media",
    ):
        # Infrastructure media adapters implement the workflows::ports::media
        # port traits directly; closing this is a later migration slice.
        return True
    if (
        path.startswith("src/workflows/")
        and len(segments) >= 2
        and segments[0] == "infrastructure"
        and segments[1] in ("config", "acquisition", "media")
    ):
        # Workflows still reference the AppConfig and DeviceSession /
        # AcquisitionPort types moved into infrastructure in this slice, and
        # the aggregate-media integration test exercises the concrete media
        # adapter it was moved alongside; closing this is a later migration
        # slice.
        return True
    return False


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
    for index, (token, offset) in enumerate(items):
        if token in roots:
            yield from expression_dependency(items, index)
        elif token == "use":
            yield from import_dependencies(normalized, index + 1, offset)


def structural_errors(path, text):
    if re.search(r"#\s*\[\s*path\s*=", text):
        yield "path-module wiring is not allowed"
    if path == "src/lib.rs":
        for name in re.findall(r"\bpub\s+mod\s+(\w+)\s*;", text):
            if name in LAYERS:
                yield "implementation layers must remain private"
    if pathlib.PurePosixPath(path).stem in FACADES and path.count("/") == 1:
        declaration = r"\b(?:fn|struct|enum|trait|impl|const|static|type|mod)\s+\w+"
        if re.search(declaration, text):
            yield "compatibility facade contains implementation"


def violations(path, source):
    items = list(tokens(source))
    forbidden = forbidden_roots(path)
    seen = set()
    for root, offset, segments in dependencies(items):
        if root not in forbidden or is_exempt(path, segments):
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
