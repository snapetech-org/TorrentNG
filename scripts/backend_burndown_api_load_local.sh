#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$ROOT"

if [[ $# -ne 1 ]]; then
    echo "usage: $0 REPORT.md" >&2
    exit 2
fi
report_path="$1"
mkdir -p -- "$(dirname -- "$report_path")"

scratch_parent="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if [[ ! -d "$scratch_parent" || ! -w "$scratch_parent" ]]; then
    echo "temporary directory is unavailable: $scratch_parent" >&2
    exit 2
fi
umask 077
task_dir="$(mktemp -d "${scratch_parent%/}/torrentng-api-load.XXXXXX")"
daemon_pid=""

cleanup() {
    if [[ -n "$daemon_pid" ]] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill -INT "$daemon_pid" 2>/dev/null || true
        for ((attempt = 0; attempt < 10; attempt += 1)); do
            if ! kill -0 "$daemon_pid" 2>/dev/null; then
                break
            fi
            sleep 1
        done
        if kill -0 "$daemon_pid" 2>/dev/null; then
            kill -TERM "$daemon_pid" 2>/dev/null || true
        fi
        wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf -- "$task_dir"
}
trap cleanup EXIT

api_port="$(python3 - <<'PY'
import socket

with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
)"

TNG_LOCAL_LOAD_DIR="$task_dir" TNG_LOCAL_LOAD_PORT="$api_port" python3 - <<'PY'
from pathlib import Path
import os
import secrets

root = Path(os.environ["TNG_LOCAL_LOAD_DIR"])
port = int(os.environ["TNG_LOCAL_LOAD_PORT"])
session = root / "session"
downloads = root / "downloads"
session.mkdir(mode=0o700)
downloads.mkdir(mode=0o700)
token = secrets.token_hex(32)
(root / "api-token").write_text(token + "\n", encoding="ascii")
(root / "api-token").chmod(0o600)
(root / "api-header").write_text(f"Authorization: Bearer {token}\n", encoding="ascii")
(root / "api-header").chmod(0o600)
config = f'''[daemon]
api_bind = "127.0.0.1:{port}"
session_dir = "{session}"
log_level = "warn"
shutdown_timeout_secs = 5

[network]
listen_port = 0
max_peers = 32

[dht]
enabled = false

[storage]
download_dir = "{downloads}"

[auth]
username = "load-test"
password = "{token}"
api_tokens = ["{token}"]
'''
config_path = root / "config.toml"
config_path.write_text(config, encoding="utf-8")
config_path.chmod(0o600)
PY

cargo build --release --locked -p torrentngd
TORRENTNGD_CONFIG="$task_dir/config.toml" \
    "$ROOT/target/release/torrentngd" >"$task_dir/daemon.log" 2>&1 &
daemon_pid=$!
api_token="$(<"$task_dir/api-token")"

base_url="http://127.0.0.1:$api_port"
ready=0
for ((attempt = 0; attempt < 30; attempt += 1)); do
    if curl --noproxy '*' --fail --silent --show-error --max-time 2 \
        --header "@$task_dir/api-header" "$base_url/health" \
        >/dev/null 2>&1; then
        ready=1
        break
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        break
    fi
    sleep 1
done
if [[ "$ready" -ne 1 ]]; then
    echo "isolated torrentngd did not become ready" >&2
    cat -- "$task_dir/daemon.log" >&2
    exit 1
fi

TNG_BASE_URL="$base_url" \
TNG_API_TOKEN="$api_token" \
TNG_LOAD_COMMIT="${TNG_LOAD_COMMIT:-${GITHUB_SHA:-$(git rev-parse HEAD)}}" \
TNG_LOAD_BINARY="$ROOT/target/release/torrentngd" \
TNG_LOAD_PID="$daemon_pid" \
    python3 "$ROOT/scripts/backend_burndown_api_load.py" "$report_path"
