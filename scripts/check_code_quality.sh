#!/usr/bin/env bash

set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

readonly rust_line_limit=600
readonly lizard_file_nloc_limit=500
readonly ruff_version="0.15.20"
readonly lizard_version="1.23.0"
readonly jscpd_version="5.1.2"

run_ruff() {
  if command -v ruff >/dev/null 2>&1; then
    ruff "$@"
  elif command -v uv >/dev/null 2>&1; then
    uvx --python 3.11 --from "ruff==$ruff_version" ruff "$@"
  else
    echo "ruff $ruff_version is required" >&2
    return 127
  fi
}

run_lizard() {
  if command -v lizard >/dev/null 2>&1; then
    lizard "$@"
  elif command -v uv >/dev/null 2>&1; then
    uvx --python 3.11 --from "lizard==$lizard_version" lizard "$@"
  else
    echo "lizard $lizard_version is required" >&2
    return 127
  fi
}

run_jscpd() {
  if [[ -n ${OPEN_SCANLINE_JSCPD:-} ]]; then
    "$OPEN_SCANLINE_JSCPD" "$@"
  elif command -v jscpd >/dev/null 2>&1; then
    jscpd "$@"
  elif command -v npx >/dev/null 2>&1; then
    npx --yes "jscpd@$jscpd_version" "$@"
  else
    echo "jscpd $jscpd_version is required" >&2
    return 127
  fi
}

check_tool_version() {
  local actual=$1
  local expected=$2
  local tool=$3
  if [[ $actual != "$expected" ]]; then
    echo "$tool version mismatch: expected '$expected', got '$actual'" >&2
    return 1
  fi
}

check_rust_file_lengths() {
  local failed=0
  local lines
  local path
  local -a roots=()

  for path in src tests examples benches; do
    [[ -d $path ]] && roots+=("$path")
  done

  while IFS= read -r -d '' path; do
    lines=$(awk 'END { print NR }' "$path")
    if ((lines > rust_line_limit)); then
      printf '%s: %d lines (limit %d)\n' "$path" "$lines" "$rust_line_limit" >&2
      failed=1
    fi
  done < <(find "${roots[@]}" -type f -name '*.rs' -print0 | sort -z)

  if [[ -f build.rs ]]; then
    lines=$(awk 'END { print NR }' build.rs)
    if ((lines > rust_line_limit)); then
      printf 'build.rs: %d lines (limit %d)\n' "$lines" "$rust_line_limit" >&2
      failed=1
    fi
  fi

  if ((failed)); then
    return 1
  fi
  echo "Rust file length check passed (limit: $rust_line_limit physical lines)"
}

check_rust_file_lengths

check_tool_version "$(run_ruff --version)" "ruff $ruff_version" ruff
run_ruff check --select E,F,C90 scripts
python3 -m unittest scripts/test_check_lizard_file_nloc.py

check_tool_version "$(run_lizard --version)" "$lizard_version" lizard
run_lizard -l rust -l python -C 8 -L 50 -a 8 -w src tests scripts
lizard_report=$(mktemp "${TMPDIR:-/tmp}/open-scanline-lizard.XXXXXX.xml")
trap 'rm -f "$lizard_report"' EXIT
run_lizard -l rust -l python -X src tests scripts >"$lizard_report"
python3 scripts/check_lizard_file_nloc.py \
  --max-nloc "$lizard_file_nloc_limit" "$lizard_report"

check_tool_version "$(run_jscpd --version)" "jscpd $jscpd_version" jscpd
run_jscpd src tests scripts --config .jscpd.json --no-colors --no-tips
