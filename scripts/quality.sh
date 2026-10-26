#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# Tests create their own repositories; a caller's explicit Git paths must not leak.
while read -r name; do
    unset "$name"
done < <(git rev-parse --local-env-vars)

status=0
check() {
    printf '\n== %s ==\n' "$1"
    shift
    "$@" || status=1
}

check 'Quality gate regression tests' python -m unittest discover -s tests/python -p test_quality.py -v
check 'Rust formatting' cargo fmt --all --check
check 'Clippy (warnings are errors)' cargo clippy --locked --all-targets --features desktop -- -D warnings
check 'Rust tests (including desktop)' cargo test --locked --all-targets --features desktop
check 'Rust complexity and duplication' bash scripts/check-metrics.sh

if (( status != 0 )); then
    echo 'Quality checks failed. Fix the findings above; no historical baseline is exempt.' >&2
fi
exit "$status"
