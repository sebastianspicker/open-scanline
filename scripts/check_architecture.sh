#!/usr/bin/env sh
# Fast structural guard for the layer boundaries documented in docs/architecture.md.
set -eu

failed=0

report_matches() {
    label=$1
    pattern=$2
    shift 2
    for file in "$@"; do
        [ -f "$file" ] || continue
        matches=$(grep -nE "$pattern" "$file" || true)
        if [ -n "$matches" ]; then
            printf '%s\n' "architecture violation: $label" >&2
            printf '%s\n' "$matches" | sed "s|^|$file:|" >&2
            failed=1
        fi
    done
}

domain_files=$(find src/domain -type f -name '*.rs' -print 2>/dev/null || true)
workflow_files=$(find src/workflows -type f -name '*.rs' -print 2>/dev/null || true)
source_files=$(find src -type f -name '*.rs' -print 2>/dev/null || true)
outer_layer_files="$(find src/infrastructure src/inbound -type f -name '*.rs' -print 2>/dev/null || true) src/composition.rs"

# `error` is a shared value boundary; domain may otherwise refer only to itself.
report_matches \
    'domain depends on an outer layer or compatibility facade' \
    'crate::(infrastructure|inbound|workflows|composition|batch|cli|config|core|device|escl|export|features|film|gui|i18n|icc|imaging|manufacturers|ml|ocr|packaging|pipeline|platform|plugin|process|sane|scan|twain|wia)(::|[^[:alnum:]_])' \
    $domain_files

report_matches \
    'workflow imports infrastructure, inbound, or a legacy facade' \
    'crate::(infrastructure|inbound|composition|batch|cli|config|core|device|escl|export|features|film|gui|i18n|icc|imaging|manufacturers|ml|ocr|packaging|pipeline|platform|plugin|process|sane|scan|twain|wia)(::|[^[:alnum:]_])' \
    $workflow_files

report_matches \
    'infrastructure, inbound, or composition imports a compatibility facade' \
    'crate::(batch|cli|config|core|device|escl|export|features|film|gui|i18n|icc|imaging|manufacturers|ml|ocr|packaging|pipeline|platform|plugin|process|sane|scan|twain|wia)(::|[^[:alnum:]_])' \
    $outer_layer_files

report_matches \
    'path-module wiring is not allowed; use normal module layout' \
    '^[[:space:]]*#\[path[[:space:]]*=' \
    $source_files

report_matches \
    'implementation layers must remain private behind compatibility facades' \
    '^[[:space:]]*pub[[:space:]]+mod[[:space:]]+(composition|domain|error|inbound|infrastructure|workflows)[[:space:]]*;' \
    src/lib.rs

# These files exist solely to preserve established module paths.
facades='src/batch.rs src/cli.rs src/config.rs src/core.rs src/device.rs src/escl.rs src/export.rs src/features.rs src/film.rs src/gui.rs src/i18n.rs src/icc.rs src/imaging.rs src/manufacturers.rs src/ml.rs src/ocr.rs src/packaging.rs src/platform.rs src/plugin.rs src/process.rs src/sane.rs src/scan.rs src/twain.rs src/wia.rs'
report_matches \
    'compatibility facade contains implementation; move it to a layer' \
    '^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?(fn|struct|enum|trait|impl|const|static|type|mod)[[:space:]]' \
    $facades

if [ "$failed" -ne 0 ]; then
    exit 1
fi

printf '%s\n' 'architecture check passed'
