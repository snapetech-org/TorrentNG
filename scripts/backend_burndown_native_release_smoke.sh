#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/curl_policy.sh
source "$ROOT/scripts/curl_policy.sh"
REPORT_DIR="${REPORT_DIR:-$ROOT/certification/reports}"
BIN="${TNG_RELEASE_BINARY:-$ROOT/target/release/torrentngd}"
CONFIG="${TNG_RELEASE_CONFIG:-}"
STATIC_DIR="${TNG_STATIC_DIR:-$ROOT/sidecar/static}"
TOKEN="${TNG_RELEASE_TOKEN:-backend-burndown-token-local-smoke}"
API_PORT="${TNG_RELEASE_API_PORT:-28080}"
if [[ ! "$API_PORT" =~ ^[0-9]{1,5}$ ]] || (( API_PORT < 1 || API_PORT > 65535 )); then
  echo "TNG_RELEASE_API_PORT must be an integer from 1 to 65535" >&2
  exit 2
fi
API_BIND="127.0.0.1:$API_PORT"
API_URL="http://$API_BIND"
OUT="${1:-$REPORT_DIR/backend-burndown-native-release-smoke-$(date -u +%Y%m%dT%H%M%SZ).md}"
PREFIX="${OUT%.md}"
smoke_tmp=""
daemon_pid=""

mkdir -p "$(dirname "$OUT")"
cleanup() {
  if [[ -n "$daemon_pid" ]] && kill -0 "$daemon_pid" 2>/dev/null; then
    kill -TERM "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  if [[ -n "$smoke_tmp" && -d "$smoke_tmp" ]]; then
    rm -rf -- "$smoke_tmp"
  fi
}
trap cleanup EXIT

if [[ -z "$CONFIG" ]]; then
  smoke_tmp="$(mktemp -d "${TMPDIR:-/tmp}/torrentng-native-release-smoke.XXXXXX")"
  CONFIG="$smoke_tmp/config.toml"
  TNG_SMOKE_TMP="$smoke_tmp" TNG_SMOKE_CONFIG="$CONFIG" TNG_RELEASE_TOKEN="$TOKEN" \
    TNG_RELEASE_API_PORT="$API_PORT" \
    python3 - <<'PY'
import json
import os
from pathlib import Path

root = Path(os.environ["TNG_SMOKE_TMP"])
quote = lambda value: json.dumps(str(value))
api_bind = f"127.0.0.1:{os.environ['TNG_RELEASE_API_PORT']}"
config = "\n".join(
    [
        "[daemon]",
        f"api_bind = {quote(api_bind)}",
        f"session_dir = {quote(root / 'session')}",
        "log_level = \"info\"",
        "",
        "[network]",
        "listen_port = 0",
        "",
        "[storage]",
        f"download_dir = {quote(root / 'downloads')}",
        "",
        "[dht]",
        "enabled = false",
        "",
        "[auth]",
        f"api_tokens = [{quote(os.environ['TNG_RELEASE_TOKEN'])}]",
        "",
    ]
)
Path(os.environ["TNG_SMOKE_CONFIG"]).write_text(config, encoding="utf-8")
PY
fi

test -x "$BIN"
test -f "$CONFIG"

LOG="${PREFIX}.log"
HEALTH="${PREFIX}.health.json"
TORRENTS="${PREFIX}.torrents.json"
TRANSFER="${PREFIX}.transfer.json"
QBIT="${PREFIX}.qbit.json"
QBIT_TRANSFER="${PREFIX}.qbit-transfer.json"
METRICS="${PREFIX}.metrics.txt"

start_epoch_ms="$(date +%s%3N)"
TORRENTNGD_CONFIG="$CONFIG" TNG_STATIC_DIR="$STATIC_DIR" "$BIN" >"$LOG" 2>&1 &
daemon_pid=$!

ready=0
for _ in $(seq 1 60); do
  if curl -fsS -H "Authorization: Bearer $TOKEN" "$API_URL/health" >"$HEALTH" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.2
done
if [[ "$ready" != "1" ]]; then
  echo "daemon did not become ready; see $LOG" >&2
  exit 1
fi

curl -fsS -H "Authorization: Bearer $TOKEN" \
  "$API_URL/api/v1/torrents?limit=2" >"$TORRENTS"
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "$API_URL/api/v1/transfer/info" >"$TRANSFER"
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "$API_URL/api/qb/v2/torrents/info?limit=2" >"$QBIT"
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "$API_URL/api/qb/v2/transfer/info" >"$QBIT_TRANSFER"
curl -fsS -H "Authorization: Bearer $TOKEN" \
  "$API_URL/metrics" >"$METRICS"

jq -e '.ready == true and .engine.subsystems.engine.alive == true and .engine.subsystems.storage_workers.healthy == true' "$HEALTH" >/dev/null
jq -e 'has("snapshot") and has("total") and has("torrents")' "$TORRENTS" >/dev/null
jq -e 'has("dl_info_speed") and has("up_info_speed")' "$TRANSFER" >/dev/null
jq -e 'type == "array"' "$QBIT" >/dev/null
jq -e 'has("connection_status") and has("dl_info_speed")' "$QBIT_TRANSFER" >/dev/null
grep -q '^torrentng_storage_workers_healthy 1$' "$METRICS"
grep -q '^torrentng_api_snapshot_refreshes_total ' "$METRICS"

kill -TERM "$daemon_pid"
shutdown_polls=0
for _ in $(seq 1 50); do
  shutdown_polls=$((shutdown_polls + 1))
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    break
  fi
  sleep 0.2
done
wait "$daemon_pid"
daemon_pid=""

build_hash="$(sha256sum "$BIN" | awk '{print $1}')"
binary_bytes="$(stat -c '%s' "$BIN")"
metrics_lines="$(wc -l <"$METRICS" | tr -d ' ')"
metrics_bytes="$(wc -c <"$METRICS" | tr -d ' ')"
duration_ms="$(( $(date +%s%3N) - start_epoch_ms ))"

if git -C "$ROOT" diff --quiet --ignore-submodules -- && git -C "$ROOT" diff --cached --quiet --ignore-submodules --; then
  worktree="clean"
else
  worktree="dirty"
fi

{
  echo "# TorrentNG Client Release-Binary Smoke"
  echo
  echo "- Date UTC: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Host: $(hostname)"
  echo "- Commit: $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unavailable)"
  echo "- Binary: $BIN"
  echo "- Binary bytes: $binary_bytes"
  echo "- Binary SHA-256: $build_hash"
  echo "- API bind: $API_BIND"
  echo "- Config: $CONFIG"
  echo "- Worktree: $worktree"
  echo
  echo "This run exercised the optimized production daemon binary with the"
  echo "authenticated TorrentNG and qBittorrent facades, then sent SIGTERM and"
  echo "waited for process exit. It is a deployment smoke test, not 100k-scale"
  echo "capacity evidence."
  echo
  echo "## Checks"
  echo
  echo "| Check | Result | Evidence |"
  echo "|---|---|---|"
  echo "| Startup and health | PASS | $(basename "$HEALTH") |"
  echo "| TorrentNG list envelope | PASS | $(basename "$TORRENTS") |"
  echo "| TorrentNG transfer info | PASS | $(basename "$TRANSFER") |"
  echo "| qBittorrent list | PASS | $(basename "$QBIT") |"
  echo "| qBittorrent transfer info | PASS | $(basename "$QBIT_TRANSFER") |"
  echo "| Prometheus metrics | PASS | $(basename "$METRICS"); $metrics_lines lines / $metrics_bytes bytes |"
  echo "| SIGTERM and clean exit | PASS | $(basename "$LOG"); $shutdown_polls polls |"
  echo
  echo "- Total smoke duration milliseconds: $duration_ms"
  echo
  echo "Overall status: PASS"
} >"$OUT"

echo "$OUT"
