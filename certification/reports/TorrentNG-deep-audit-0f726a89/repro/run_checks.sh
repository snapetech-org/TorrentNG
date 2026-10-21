#!/usr/bin/env bash
set -euo pipefail
audit_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
audit_build="$(mktemp -d)"
trap 'rm -rf -- "$audit_build"' EXIT
audit_rustc="${RUSTC:-rustc}"
"$audit_rustc" --version
"$audit_rustc" --edition=2021 -O "$audit_root/repro/rate_repro.rs" -o "$audit_build/rate_repro"
"$audit_build/rate_repro"
"$audit_rustc" --edition=2021 -O --test "$audit_root/repro/rate_repro.rs" -o "$audit_build/rate_tests"
"$audit_build/rate_tests"
"$audit_rustc" --edition=2021 -O "$audit_root/repro/queue_cost.rs" -o "$audit_build/queue_cost"
"$audit_build/queue_cost"
