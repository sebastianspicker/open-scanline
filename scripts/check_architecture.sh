#!/usr/bin/env sh
# Structural guard for the boundaries documented in docs/architecture.md.
set -eu
cd "$(dirname "$0")/.."
python3 -m unittest discover -s scripts -p 'test_check_architecture.py'
python3 scripts/check_architecture.py
