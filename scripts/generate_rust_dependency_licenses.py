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
PackageRecord = tuple[str, str, str, Path]


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
        line = ANSI_ESCAPE.sub("", raw_line)
        line = line.removesuffix(" (*)").removesuffix(" (proc-macro)")
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
            f"missing cached sources for {name}@{version}; "
            "run `cargo fetch --locked` first"
        )
    return archive


def is_license_file(path: str) -> bool:
    return Path(path).name.lower().startswith(LICENSE_FILE_PREFIXES)


def source_license_files(
    manifest_path: Path, name: str, version: str
) -> list[tuple[str, bytes]]:
    source_dir = source_directory(manifest_path, name, version)
    if source_dir.is_dir():
        return directory_license_files(source_dir)
    return archive_license_files(cached_crate_archive(manifest_path, name, version))


def source_directory(manifest_path: Path, name: str, version: str) -> Path:
    return (
        Path(VENDORED_SOURCES) / f"{name}-{version}"
        if VENDORED_SOURCES is not None
        else manifest_path.parent
    )


def directory_license_files(source_dir: Path) -> list[tuple[str, bytes]]:
    files = [
        path
        for path in source_dir.rglob("*")
        if path.is_file() and is_license_file(path.name)
    ]
    return [
        (path.relative_to(source_dir).as_posix(), path.read_bytes())
        for path in sorted(files)
    ]


def archive_license_files(archive: Path) -> list[tuple[str, bytes]]:
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


def verify_vendored_sources(package_records: list[PackageRecord]) -> None:
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
            "vendored source directory is incomplete for the locked "
            "all-target closure: "
            + ", ".join(missing)
        )


def write_text(output: io.BytesIO, value: str) -> None:
    output.write(value.encode("utf-8"))


def markdown_fence(text: str) -> str:
    runs = (len(match.group(0)) for match in re.finditer(r"~+", text))
    longest = max(runs, default=0)
    return "~" * max(4, longest + 1)


def package_metadata() -> tuple[
    dict[tuple[str, str], dict[str, object]], tuple[str, str]
]:
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
    return by_key, root_key


def dependency_records(
    by_key: dict[tuple[str, str], dict[str, object]],
    root_key: tuple[str, str],
) -> tuple[set[tuple[str, str]], list[PackageRecord]]:
    closure = tree_packages()
    if root_key not in closure:
        raise RuntimeError(
            "cargo tree all-target closure did not include the root package"
        )
    dependency_keys = sorted(closure - {root_key})
    missing = [key for key in dependency_keys if key not in by_key]
    if missing:
        raise RuntimeError(f"cargo metadata is missing tree packages: {missing}")
    records = []
    for name, version in dependency_keys:
        package = by_key[(name, version)]
        license_expression = package.get("license") or "NOASSERTION"
        manifest_path = Path(package["manifest_path"])
        records.append((name, version, str(license_expression), manifest_path))
    return closure, records


def write_bundle_header(
    output: io.BytesIO, dependency_count: int, closure_count: int
) -> None:
    write_text(output, "# Rust dependency license bundle\n\n")
    write_text(
        output,
        "This bundle records the third-party packages reported by "
        "`cargo tree --locked --all-features --target all -e normal` "
        f"for the locked all-feature, all-target build ({dependency_count} packages; "
        f"{closure_count} entries including open-scanline). It includes every "
        "locally cached crate file named LICENSE*, COPYING*, NOTICE*, or COPYRIGHT* "
        "for that closure. Package license declarations below are copied from each "
        "cached Cargo.toml; source file contents are reproduced below as Markdown "
        "text.\n\n",
    )
    write_text(
        output,
        "The portable ZIP embeds this file verbatim. It is a source-metadata record, "
        "not a statement about legal obligations or the license selected by a "
        "downstream distributor.\n\n",
    )


def write_package_inventory(
    output: io.BytesIO, package_records: list[PackageRecord]
) -> None:
    write_text(
        output,
        "## Package inventory\n\n| Package | Declared license |\n| --- | --- |\n",
    )
    for name, version, license_expression, _manifest_path in package_records:
        write_text(output, f"| `{name}@{version}` | `{license_expression}` |\n")


def write_missing_license_notice(
    output: io.BytesIO, name: str, version: str, license_expression: str
) -> None:
    write_text(output, f"\n### {name}@{version}: no source license file\n\n")
    write_text(
        output,
        f"Declared license metadata: `{license_expression}`. The cached package "
        "contains no file named LICENSE*, COPYING*, NOTICE*, or COPYRIGHT*; its "
        "declaration is retained in the package inventory above.\n",
    )


def write_license_file(
    output: io.BytesIO,
    package: tuple[str, str, str],
    relative_path: str,
    contents: bytes,
) -> None:
    name, version, license_expression = package
    try:
        text = contents.decode("utf-8")
    except UnicodeDecodeError as error:
        raise RuntimeError(
            f"license file for {name}@{version} is not UTF-8: {relative_path}"
        ) from error
    fence = markdown_fence(text)
    write_text(output, f"\n### {name}@{version}: `{relative_path}`\n\n")
    write_text(
        output,
        f"Declared license metadata: `{license_expression}`.\n\n{fence}text\n",
    )
    write_text(output, text)
    if not text.endswith("\n"):
        write_text(output, "\n")
    write_text(output, f"{fence}\n")


def write_source_licenses(
    output: io.BytesIO, package_records: list[PackageRecord]
) -> int:
    write_text(output, "\n## Source license and notice files\n")
    verify_vendored_sources(package_records)
    source_file_count = 0
    for name, version, license_expression, manifest_path in package_records:
        files = source_license_files(manifest_path, name, version)
        source_file_count += len(files)
        if not files:
            write_missing_license_notice(output, name, version, license_expression)
            continue
        package = (name, version, license_expression)
        for relative_path, contents in files:
            write_license_file(output, package, relative_path, contents)
    return source_file_count


def bundle_contents() -> tuple[bytes, int, int]:
    by_key, root_key = package_metadata()
    closure, package_records = dependency_records(by_key, root_key)

    output = io.BytesIO()
    write_bundle_header(output, len(package_records), len(closure))
    write_package_inventory(output, package_records)
    source_file_count = write_source_licenses(output, package_records)
    return output.getvalue(), len(closure), source_file_count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument(
        "--write", action="store_true", help="regenerate the tracked bundle"
    )
    modes.add_argument(
        "--check", action="store_true", help="fail if the tracked bundle is stale"
    )
    args = parser.parse_args()

    try:
        contents, closure_count, source_file_count = bundle_contents()
    except (
        OSError,
        RuntimeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
    ) as error:
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
            f"{hashlib.sha256(contents).hexdigest()} ({len(contents)} bytes), "
            "got sha256="
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
