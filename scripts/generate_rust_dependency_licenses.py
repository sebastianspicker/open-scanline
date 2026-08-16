#!/usr/bin/env python3
"""Generate or verify the portable all-feature, all-target Rust license bundle."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile


ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / "assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md"
LICENSE_FILE_PREFIXES = ("license", "copying", "notice", "copyright")
TREE_LINE = re.compile(r"^([A-Za-z0-9][A-Za-z0-9_.+-]*) v([^\s]+)(?: \([^)]*\))?$")
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
VENDORED_SOURCES = os.environ.get("OPEN_SCANLINE_LICENSE_SOURCE_DIR")


def cargo_json(*args: str) -> dict[str, object]:
    completed = subprocess.run(
        ["cargo", *args],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return json.loads(completed.stdout)


def tree_packages() -> set[tuple[str, str]]:
    completed = subprocess.run(
        [
            "cargo",
            "tree",
            "--locked",
            "--all-features",
            "--target",
            "all",
            "-e",
            "normal",
            "--prefix",
            "none",
        ],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    packages = set()
    for raw_line in completed.stdout.splitlines():
        line = ANSI_ESCAPE.sub("", raw_line).removesuffix(" (*)").removesuffix(" (proc-macro)")
        match = TREE_LINE.fullmatch(line)
        if match is None:
            raise RuntimeError(f"could not parse cargo tree line: {raw_line!r}")
        packages.add((match.group(1), match.group(2)))
    return packages


def cached_crate_archive(manifest_path: Path, name: str, version: str) -> Path:
    registry_hash = manifest_path.parent.parent.name
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    archive = cargo_home / "registry/cache" / registry_hash / f"{name}-{version}.crate"
    if not archive.is_file():
        raise RuntimeError(
            f"missing cached sources for {name}@{version}; run `cargo fetch --locked` first"
        )
    return archive


def is_license_file(path: str) -> bool:
    return Path(path).name.lower().startswith(LICENSE_FILE_PREFIXES)


def source_license_files(manifest_path: Path, name: str, version: str) -> list[tuple[str, bytes]]:
    source_dir = (
        Path(VENDORED_SOURCES) / f"{name}-{version}"
        if VENDORED_SOURCES is not None
        else manifest_path.parent
    )
    if source_dir.is_dir():
        files = [
            path
            for path in source_dir.rglob("*")
            if path.is_file() and is_license_file(path.name)
        ]
        return [
            (path.relative_to(source_dir).as_posix(), path.read_bytes())
            for path in sorted(files)
        ]

    archive = cached_crate_archive(manifest_path, name, version)
    with tarfile.open(archive, "r:gz") as contents:
        files = []
        for member in contents.getmembers():
            parts = Path(member.name).parts
            if member.isfile() and is_license_file(member.name):
                source = contents.extractfile(member)
                if source is None:
                    raise RuntimeError(f"could not read {member.name} from {archive}")
                files.append((Path(*parts[1:]).as_posix(), source.read()))
    return sorted(files)


def verify_vendored_sources(package_records: list[tuple[str, str, str, Path]]) -> None:
    if VENDORED_SOURCES is None:
        return
    source_root = Path(VENDORED_SOURCES)
    if not source_root.is_dir():
        raise RuntimeError(
            "OPEN_SCANLINE_LICENSE_SOURCE_DIR is not a directory: "
            f"{source_root}"
        )
    missing = [
        f"{name}@{version}"
        for name, version, _license, _manifest in package_records
        if not (source_root / f"{name}-{version}").is_dir()
    ]
    if missing:
        raise RuntimeError(
            "vendored source directory is incomplete for the locked all-target closure: "
            + ", ".join(missing)
        )


def write_text(output: io.BytesIO, value: str) -> None:
    output.write(value.encode("utf-8"))


def markdown_fence(text: str) -> str:
    longest = max((len(match.group(0)) for match in re.finditer(r"~+", text)), default=0)
    return "~" * max(4, longest + 1)


def bundle_contents() -> tuple[bytes, int, int]:
    metadata = cargo_json("metadata", "--locked", "--format-version", "1")
    packages = metadata["packages"]
    if not isinstance(packages, list):
        raise RuntimeError("cargo metadata did not return packages")
    by_key = {
        (package["name"], package["version"]): package
        for package in packages
        if isinstance(package, dict)
    }
    resolve = metadata["resolve"]
    if not isinstance(resolve, dict) or not isinstance(resolve.get("root"), str):
        raise RuntimeError("cargo metadata did not identify the root package")
    root_id = resolve["root"]
    root = next((package for package in packages if package.get("id") == root_id), None)
    if not isinstance(root, dict):
        raise RuntimeError("could not find the root package in cargo metadata")
    root_key = (root["name"], root["version"])

    closure = tree_packages()
    if root_key not in closure:
        raise RuntimeError("cargo tree all-target closure did not include the root package")
    dependency_keys = sorted(closure - {root_key})
    missing = [key for key in dependency_keys if key not in by_key]
    if missing:
        raise RuntimeError(f"cargo metadata is missing tree packages: {missing}")

    output = io.BytesIO()
    write_text(output, "# Rust dependency license bundle\n\n")
    write_text(
        output,
        "This bundle records the third-party packages reported by "
        "`cargo tree --locked --all-features --target all -e normal` "
        f"for the locked all-feature, all-target build ({len(dependency_keys)} packages; "
        f"{len(closure)} entries including open-scanline). It includes every locally cached "
        "crate file named LICENSE*, COPYING*, NOTICE*, or COPYRIGHT* for that closure. "
        "Package license declarations below are copied from each cached Cargo.toml; "
        "source file contents are reproduced below as Markdown text.\n\n",
    )
    write_text(
        output,
        "The portable ZIP embeds this file verbatim. It is a source-metadata record, not a "
        "statement about legal obligations or the license selected by a downstream distributor.\n\n",
    )
    write_text(output, "## Package inventory\n\n| Package | Declared license |\n| --- | --- |\n")
    package_records = []
    for name, version in dependency_keys:
        package = by_key[(name, version)]
        license_expression = package.get("license") or "NOASSERTION"
        manifest_path = Path(package["manifest_path"])
        package_records.append((name, version, str(license_expression), manifest_path))
        write_text(output, f"| `{name}@{version}` | `{license_expression}` |\n")

    write_text(output, "\n## Source license and notice files\n")
    verify_vendored_sources(package_records)
    source_file_count = 0
    for name, version, license_expression, manifest_path in package_records:
        files = source_license_files(manifest_path, name, version)
        source_file_count += len(files)
        if not files:
            write_text(output, f"\n### {name}@{version}: no source license file\n\n")
            write_text(
                output,
                f"Declared license metadata: `{license_expression}`. The cached package contains "
                "no file named LICENSE*, COPYING*, NOTICE*, or COPYRIGHT*; its declaration is "
                "retained in the package inventory above.\n",
            )
            continue
        for relative_path, contents in files:
            try:
                text = contents.decode("utf-8")
            except UnicodeDecodeError as error:
                raise RuntimeError(
                    f"license file for {name}@{version} is not UTF-8: {relative_path}"
                ) from error
            fence = markdown_fence(text)
            write_text(output, f"\n### {name}@{version}: `{relative_path}`\n\n")
            write_text(output, f"Declared license metadata: `{license_expression}`.\n\n{fence}text\n")
            write_text(output, text)
            if not text.endswith("\n"):
                write_text(output, "\n")
            write_text(output, f"{fence}\n")
    return output.getvalue(), len(closure), source_file_count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--write", action="store_true", help="regenerate the tracked bundle")
    modes.add_argument("--check", action="store_true", help="fail if the tracked bundle is stale")
    args = parser.parse_args()

    try:
        contents, closure_count, source_file_count = bundle_contents()
    except (OSError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        print(f"license bundle generation failed: {error}", file=sys.stderr)
        return 2

    if args.write:
        temporary = BUNDLE.with_suffix(BUNDLE.suffix + ".tmp")
        temporary.write_bytes(contents)
        temporary.replace(BUNDLE)
        print(
            f"wrote {BUNDLE.relative_to(ROOT)}: closure={closure_count}, "
            f"third_party={closure_count - 1}, source_files={source_file_count}"
        )
        return 0

    current = BUNDLE.read_bytes() if BUNDLE.is_file() else b""
    if current != contents:
        print(
            f"stale {BUNDLE.relative_to(ROOT)}: expected sha256="
            f"{hashlib.sha256(contents).hexdigest()} ({len(contents)} bytes), got sha256="
            f"{hashlib.sha256(current).hexdigest()} ({len(current)} bytes); "
            "run `python3 scripts/generate_rust_dependency_licenses.py --write`",
            file=sys.stderr,
        )
        return 1
    print(
        f"verified {BUNDLE.relative_to(ROOT)}: closure={closure_count}, "
        f"third_party={closure_count - 1}, source_files={source_file_count}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
