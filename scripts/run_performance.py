#!/usr/bin/env python3
"""Build once, then measure deterministic workloads without competing builds."""

import argparse
import json
import hashlib
import os
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def command_output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def build(features):
    args = [
        "cargo",
        "build",
        "--release",
        "--locked",
        "--no-default-features",
        "--example",
        "optimization",
    ]
    if features:
        args.extend(["--features", features])
    subprocess.run(args, cwd=ROOT, check=True)


def source_fingerprint():
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
    ).decode().split("\0")
    digest = hashlib.sha256()
    for name in sorted(set(paths)):
        if name.endswith(".rs") or name in (
            "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"
        ):
            path = ROOT / name
            if path.is_file():
                digest.update(name.encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def measure(args):
    if not args.skip_build:
        build(args.features)
    environment = os.environ.copy()
    if args.operations:
        environment["OPEN_SCANLINE_BENCH_OPERATIONS"] = args.operations
    if args.median:
        environment["OPEN_SCANLINE_BENCH_MEDIAN"] = "1"
    if args.media:
        environment["OPEN_SCANLINE_BENCH_MEDIA"] = "1"
    result = subprocess.check_output(
        [str(args.binary.resolve())], cwd=ROOT, text=True, env=environment
    )
    report = json.loads(result)
    report["environment"] = {
        "rustc": command_output("rustc", "--version"),
        "platform": platform.platform(),
        "revision": command_output("git", "rev-parse", "HEAD"),
        "dirty": bool(command_output("git", "status", "--porcelain")),
        "cargo_features": args.features or "core-only",
        "source_sha256": source_fingerprint(),
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "benchmark_binary_bytes": args.binary.stat().st_size,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(args.output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target/optimization/current.json"
    )
    parser.add_argument("--features", default="")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--median", action="store_true")
    parser.add_argument("--media", action="store_true")
    parser.add_argument("--operations", help="comma-separated workload names")
    suffix = ".exe" if os.name == "nt" else ""
    parser.add_argument(
        "--binary",
        type=Path,
        default=ROOT / f"target/release/examples/optimization{suffix}",
    )
    measure(parser.parse_args())


if __name__ == "__main__":
    main()
