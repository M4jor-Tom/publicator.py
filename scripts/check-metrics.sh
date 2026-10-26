#!/usr/bin/env bash
set -euo pipefail

# Paths may be supplied for the fixture tests; normal scans cover all Rust code.
if (( $# == 0 )); then
    set -- src tests examples
fi
args=()
for path in "$@"; do
    if [[ ! -e "$path" ]]; then
        echo "Missing scan path: $path" >&2
        exit 1
    fi
    args+=(--paths "$path")
done

report=$(mktemp -d)
trap 'rm -rf "$report"' EXIT
rust-code-analysis-cli --metrics --output-format json --include '*.rs' "${args[@]}" > "$report/metrics.json"
jq -s '
    if length == 0 then error("No Rust metrics produced") else . end
    | [ .[] as $file
        | if $file.kind != "unit" or ($file.name | type) != "string"
          then error("Invalid Rust metrics report") else $file end
        | recurse(.spaces[])
        | if (.metrics.cognitive.sum | type) != "number" or (.metrics.cyclomatic.sum | type) != "number"
          then error("Missing complexity metrics") else . end
        | select(.kind == "function" or .kind == "closure")
        | {file: $file.name, line: .start_line, name: .name,
           cognitive: (.metrics.cognitive.sum - ([.spaces[].metrics.cognitive.sum] | add // 0)),
           cyclomatic: (.metrics.cyclomatic.sum - ([.spaces[].metrics.cyclomatic.sum] | add // 0))}
        | select(.cognitive > 15 or .cyclomatic > 10)
      ]
' "$report/metrics.json" > "$report/violations.json"

status=0
jq -r '.[] | "\(.file):\(.line): \(.name): cognitive=\(.cognitive) (max 15), cyclomatic=\(.cyclomatic) (max 10)"' "$report/violations.json"
jq -e 'length == 0' "$report/violations.json" > /dev/null || status=1
# jscpd compares end_line - start_line: a span of 9 covers 10 inclusive lines.
jscpd --format rust --mode weak --min-tokens 100 --min-lines 9 \
    --exit-code 1 --fail-on-empty --no-gitignore --no-colors \
    --reporters console "$@" || status=1
exit "$status"
