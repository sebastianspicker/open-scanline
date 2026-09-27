#!/usr/bin/env python3
"""Reject Lizard XML reports containing files above the NLOC limit."""

from __future__ import annotations

import argparse
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


class ReportError(ValueError):
    """Raised when a Lizard XML report does not have the expected schema."""


def file_nlocs(xml_text: str) -> list[tuple[str, int]]:
    """Return file paths and NLOC values from one Lizard XML report."""
    try:
        root = ET.fromstring(xml_text)
    except ET.ParseError as error:
        raise ReportError(f"malformed Lizard XML: {error}") from error
    measures = root.findall("./measure[@type='File']")
    if len(measures) != 1:
        raise ReportError("Lizard XML must contain exactly one File measure")
    return _measure_file_nlocs(measures[0])


def _measure_file_nlocs(measure: ET.Element) -> list[tuple[str, int]]:
    labels = [label.text or "" for label in measure.findall("./labels/label")]
    if "NCSS" not in labels:
        raise ReportError("Lizard File measure is missing the NCSS label")
    nloc_index = labels.index("NCSS")
    return [_file_item_nloc(item, nloc_index) for item in measure.findall("./item")]


def _file_item_nloc(item: ET.Element, nloc_index: int) -> tuple[str, int]:
    values = item.findall("./value")
    path = item.get("name")
    if path is None or nloc_index >= len(values) or values[nloc_index].text is None:
        raise ReportError("Lizard File item is missing its path or NCSS value")
    try:
        return path, int(values[nloc_index].text)
    except ValueError as error:
        raise ReportError(f"invalid NCSS value for {path}") from error


def violations(rows: list[tuple[str, int]], maximum: int) -> list[tuple[str, int]]:
    """Return deterministic path-sorted file-limit violations."""
    return sorted((path, nloc) for path, nloc in rows if nloc > maximum)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--max-nloc", type=int, default=500)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        rows = file_nlocs(args.report.read_text(encoding="utf-8"))
    except (OSError, ReportError) as error:
        print(f"Lizard file NLOC check failed: {error}", file=sys.stderr)
        return 2
    failed = violations(rows, args.max_nloc)
    for path, nloc in failed:
        print(f"{path}: {nloc} NLOC (limit {args.max_nloc})", file=sys.stderr)
    if failed:
        return 1
    print(f"Lizard file NLOC check passed (limit: {args.max_nloc})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
