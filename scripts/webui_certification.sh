#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_DIR="${TNG_WEBUI_REPORT_DIR:-$ROOT/certification/reports}"
OUT="${1:-${TNG_WEBUI_REPORT:-$REPORT_DIR/webui-certification-$(date -u +%Y%m%dT%H%M%SZ).md}}"
# Keep this digest aligned with the WebUI browser step in .github/workflows/ci.yml.
PLAYWRIGHT_IMAGE="mcr.microsoft.com/playwright@sha256:2f05f8a734509a7b3f26a912639488ee08f58a7b292d14d62271bcff9038768f"
browser_results=""

worktree_state() {
  if git -C "$ROOT" diff --quiet && git -C "$ROOT" diff --cached --quiet &&
    [[ -z "$(git -C "$ROOT" ls-files --others --exclude-standard)" ]]; then
    printf 'clean\n'
  else
    printf 'dirty\n'
  fi
}

mkdir -p "$REPORT_DIR"
browser_results="$(mktemp -d "$REPORT_DIR/webui-browser-results.XXXXXX")"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

run_gate() {
  local name="$1"
  shift
  local log
  log="$tmpdir/$(printf '%s' "$name" | tr -c 'A-Za-z0-9_.-' '_').log"
  if "$@" >"$log" 2>&1; then
    printf '| %s | PASS |\n' "$name" >>"$OUT"
  else
    printf '| %s | FAIL |\n' "$name" >>"$OUT"
    overall=1
  fi
  {
    echo
    echo "## $name"
    echo
    echo '```text'
    cat "$log"
    echo '```'
  } >>"$OUT"
}

overall=0

dependencies_ready() {
  [[ -x "$ROOT/webui/node_modules/.bin/tsc" ]] &&
    [[ -x "$ROOT/webui/node_modules/.bin/eslint" ]] &&
    [[ -x "$ROOT/webui/node_modules/.bin/playwright" ]]
}

{
  echo "# TorrentNG WebUI Certification"
  echo
  echo "- Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Commit: $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo "- Worktree state: $(worktree_state)"
  echo "- 15k first-visible threshold: ${TNG_WEBUI_FIRST_VISIBLE_MS:-8000}ms"
  echo "- Browser evidence: $browser_results"
  echo
  echo "| Gate | Result |"
  echo "| --- | --- |"
} >"$OUT"

if ! dependencies_ready; then
  run_gate "webui dependency install" npm --prefix "$ROOT/webui" ci
fi
run_gate "webui production build" npm --prefix "$ROOT/webui" run build
run_gate "webui lint" npm --prefix "$ROOT/webui" run lint
run_gate "webui browser matrix" docker run --rm --init --ipc=host \
  -e CI=1 \
  -e DBUS_SESSION_BUS_ADDRESS=unix:path=/dev/null \
  --mount "type=bind,source=$ROOT,target=/work" \
  --mount "type=bind,source=$browser_results,target=/results" \
  --workdir /work/webui \
  "$PLAYWRIGHT_IMAGE" npm run test:e2e -- --reporter=list --output=/results

{
  echo
  if [[ "$overall" -eq 0 ]]; then
    echo "Overall status: PASS"
  else
    echo "Overall status: FAIL"
  fi
} >>"$OUT"

echo "$OUT"
exit "$overall"
