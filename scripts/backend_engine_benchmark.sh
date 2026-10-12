#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${TNG_BENCH_REPORT:-$ROOT/certification/reports/backend-engine-benchmark-$STAMP.md}"
STORAGE_ROOT="${TNG_BENCH_STORAGE_ROOT:-}"
SCRATCH_PARENT="${TNG_BENCH_SCRATCH_ROOT:-${TMPDIR:-/var/tmp}}"
API_PORT="${TNG_BENCH_API_PORT:-28182}"
PEER_PORT="${TNG_BENCH_PEER_PORT:-44445}"
TIMEOUT_SECS="${TNG_BENCH_TIMEOUT_SECS:-1200}"
SAMPLE_SECS="${TNG_BENCH_SAMPLE_SECS:-2}"
DOWNLOAD_LIMIT="${TNG_BENCH_DOWNLOAD_LIMIT_BPS:-2097152}"
UPLOAD_LIMIT="${TNG_BENCH_UPLOAD_LIMIT_BPS:-1048576}"
TORRENT_URL="${TNG_BENCH_TORRENT_URL:-https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent}"
TORRENT_NAME="${TNG_BENCH_TORRENT_NAME:-debian-13.7.0-amd64-netinst.iso}"
SHA256SUMS_URL="${TNG_BENCH_SHA256SUMS_URL:-https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/SHA256SUMS}"
SOURCE_REVISION="${TNG_BENCH_SOURCE_REVISION:-$(git -C "$ROOT" rev-parse --verify HEAD 2>/dev/null || echo unknown)}"
SOURCE_TREE="${TNG_BENCH_SOURCE_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo unknown)}"
BIN="${TNG_BENCH_BIN:-$ROOT/target/release/torrentngd}"

usage() {
  cat <<'USAGE'
usage: scripts/backend_engine_benchmark.sh

Runs TorrentNG's real-device storage matrix, then starts an isolated native
daemon in disposable scratch space and downloads one official Linux ISO while
sampling the daemon's CPU, RSS, and block-I/O counters. The API binds to
127.0.0.1. The temporary peer port is controlled by TNG_BENCH_PEER_PORT.

Required:
  TNG_BENCH_STORAGE_ROOT       Existing physical storage mount to probe
  TNG_BENCH_BIN                Release torrentngd built from this checkout

Optional:
  TNG_BENCH_REPORT             Persistent Markdown report path
  TNG_BENCH_SCRATCH_ROOT       Temporary filesystem (default: /var/tmp)
  TNG_BENCH_API_PORT           Local API port (default: 28182)
  TNG_BENCH_PEER_PORT          Peer/DHT port (default: 44445)
  TNG_BENCH_TIMEOUT_SECS       ISO completion limit (default: 1200)
  TNG_BENCH_SAMPLE_SECS        Process sample interval (default: 2)
  TNG_BENCH_DOWNLOAD_LIMIT_BPS Download cap in bytes/sec (default: 2097152)
  TNG_BENCH_UPLOAD_LIMIT_BPS   Upload cap in bytes/sec (default: 1048576)
  TNG_BENCH_TORRENT_URL        Official .torrent URL
  TNG_BENCH_TORRENT_NAME       Single-file ISO name
  TNG_BENCH_SHA256SUMS_URL     Official SHA256SUMS URL
  TNG_BENCH_SOURCE_REVISION    Source revision to record when run from an archive
  TNG_BENCH_SOURCE_TREE        Source tree hash when run from an archive

The storage report and this summary remain after the run. Download data,
daemon state, configuration, and sampler files are removed by the exit trap.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 127
  }
}

for cmd in awk chmod curl date df findmnt git grep hostname iostat ionice jq lsblk mkdir nice nproc paste ps python3 realpath rg rm sed seq sha256sum sleep ss tail uname wc; do
  need "$cmd"
done

[[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" && ! -L "$STORAGE_ROOT" ]] || {
  echo "set TNG_BENCH_STORAGE_ROOT to an existing non-symlink physical mount" >&2
  exit 2
}
STORAGE_ROOT="$(realpath -e -- "$STORAGE_ROOT")"
[[ "$STORAGE_ROOT" != / && -x "$STORAGE_ROOT" && -w "$STORAGE_ROOT" ]] || {
  echo "storage root must be writable and cannot be /" >&2
  exit 2
}
[[ -x "$BIN" ]] || { echo "release binary is not executable: $BIN" >&2; exit 2; }
[[ "$API_PORT" =~ ^[0-9]{1,5}$ && "$API_PORT" -ge 1024 && "$API_PORT" -le 65535 ]] || exit 2
[[ "$PEER_PORT" =~ ^[0-9]{1,5}$ && "$PEER_PORT" -ge 1024 && "$PEER_PORT" -le 65535 ]] || exit 2
[[ "$API_PORT" != "$PEER_PORT" ]] || { echo "API and peer ports must differ" >&2; exit 2; }
[[ "$TIMEOUT_SECS" =~ ^[0-9]{1,5}$ && "$TIMEOUT_SECS" -ge 60 && "$TIMEOUT_SECS" -le 7200 ]] || exit 2
[[ "$SAMPLE_SECS" =~ ^[1-9][0-9]?$ ]] || exit 2
[[ "$DOWNLOAD_LIMIT" =~ ^[0-9]+$ && "$UPLOAD_LIMIT" =~ ^[0-9]+$ ]] || exit 2
[[ "$TORRENT_NAME" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]{0,180}$ ]] || exit 2

port_busy() {
  local port="$1"
  ss -H -ltn "sport = :$port" | grep -q . || ss -H -lun "sport = :$port" | grep -q .
}
if port_busy "$API_PORT" || port_busy "$PEER_PORT"; then
  echo "API or peer port is already in use; set TNG_BENCH_API_PORT/TNG_BENCH_PEER_PORT" >&2
  exit 2
fi

mkdir -p "$(dirname "$OUT")"
SCRATCH_PARENT="$(realpath -e -- "$SCRATCH_PARENT")"
TMP_DIR="$(mktemp -d "$SCRATCH_PARENT/torrentng-engine-benchmark.XXXXXX")"
chmod 700 "$TMP_DIR"
DATA_DIR="$TMP_DIR/data"
SESSION_DIR="$TMP_DIR/session"
mkdir -m 700 "$DATA_DIR" "$SESSION_DIR"
STORAGE_SCRATCH="$(mktemp -d "$STORAGE_ROOT/.torrentng-engine-benchmark.XXXXXX")"
chmod 700 "$STORAGE_SCRATCH"
IOSTAT_BEFORE="$TMP_DIR/iostat-before.txt"
iostat -dx 1 2 >"$IOSTAT_BEFORE" 2>&1 || true
SAMPLES="$TMP_DIR/process.csv"
POLL_LOG="$TMP_DIR/torrent-poll.jsonl"
METRICS_LOG="$TMP_DIR/metrics-before.txt"
METRICS_POLL="$TMP_DIR/metrics-poll.tsv"
CONFIG="$TMP_DIR/torrentngd.toml"
TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
PID=""
SAMPLER_PID=""
STORAGE_STATUS="NOT_RUN"
TRANSFER_STATUS="NOT_RUN"
HASH_STATUS="NOT_RUN"

cleanup() {
  local exit_status=$?
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill -TERM "$PID" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$PID" 2>/dev/null || break
      sleep 1
    done
    kill -KILL "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  if [[ -n "$SAMPLER_PID" ]]; then
    wait "$SAMPLER_PID" 2>/dev/null || true
  fi
  if [[ -d "$STORAGE_SCRATCH" && ! -L "$STORAGE_SCRATCH" ]]; then
    rm -rf -- "$STORAGE_SCRATCH"
  fi
  if [[ -d "$TMP_DIR" && ! -L "$TMP_DIR" ]]; then
    rm -rf -- "$TMP_DIR"
  fi
  if [[ -f "$OUT" ]]; then
    {
      echo
      echo "## Cleanup"
      echo
      echo "- Torrent/data/config/session scratch removed: $([[ ! -e "$TMP_DIR" ]] && echo yes || echo no)"
      echo "- Storage benchmark scratch removed: $([[ ! -e "$STORAGE_SCRATCH" ]] && echo yes || echo no)"
      echo "- Storage result: $STORAGE_STATUS; ISO result: $TRANSFER_STATUS; checksum result: $HASH_STATUS"
      echo "- Exit status: $exit_status"
    } >>"$OUT"
  fi
  return "$exit_status"
}
trap cleanup EXIT

cat >"$CONFIG" <<EOF
[daemon]
api_bind = "127.0.0.1:$API_PORT"
session_dir = "$SESSION_DIR"
log_level = "warn"
shutdown_timeout_secs = 10

[network]
listen_port = $PEER_PORT
max_peers = 32
upload_rate_limit = $UPLOAD_LIMIT
download_rate_limit = $DOWNLOAD_LIMIT

[storage]
download_dir = "$DATA_DIR"

[db]
path = "$SESSION_DIR/state.db"

[auth]
api_tokens = ["$TOKEN"]
EOF
chmod 600 "$CONFIG"

STORAGE_REPORT="$(dirname "$OUT")/storage-hardware-$STAMP.md"
{
  echo "# TorrentNG Backend Engine Resource Benchmark"
  echo
  echo "- Started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Host: $(hostname -f 2>/dev/null || hostname)"
  echo "- Source revision: $SOURCE_REVISION"
  echo "- Source tree: $SOURCE_TREE"
  echo "- Binary: $BIN ($("$BIN" --version 2>&1 | head -1))"
  echo "- Binary SHA-256: $(sha256sum "$BIN" | awk '{print $1}')"
  echo "- Kernel: $(uname -srmo)"
  echo "- Host CPU threads: $(nproc --all)"
  echo "- Benchmark process CPU affinity: $(awk '/Cpus_allowed_list/ {print $2}' /proc/self/status)"
  echo "- RAM: $(awk '/MemTotal/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
  echo "- Host load at start: $(awk '{print $1 ", " $2 ", " $3}' /proc/loadavg)"
  echo "- Available RAM at start: $(awk '/MemAvailable/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
  echo "- Scratch filesystem: $(findmnt -n -T "$SCRATCH_PARENT" -o SOURCE,FSTYPE)"
  echo "- Storage target filesystem: $(findmnt -n -T "$STORAGE_ROOT" -o SOURCE,FSTYPE)"
  echo "- Storage target devices: $(lsblk -s -n -o NAME,ROTA "$(findmnt -n -T "$STORAGE_ROOT" -o SOURCE | sed 's/\[.*//')" 2>/dev/null | paste -sd ',' - || echo unknown)"
  echo "- Temporary API bind: 127.0.0.1:$API_PORT"
  echo "- Temporary peer/DHT port: $PEER_PORT"
  echo "- Max peers: 32"
  echo "- Download/upload caps: $DOWNLOAD_LIMIT/$UPLOAD_LIMIT bytes/sec"
  echo "- Process scheduling: child commands request nice +10 and idle I/O priority; inherited niceness can raise the effective value"
  echo "- Public source: $TORRENT_URL"
  echo "- ISO filename: $TORRENT_NAME"
  echo "- Completion timeout: ${TIMEOUT_SECS}s"
  echo
  echo "## Storage benchmark"
  echo
  echo "The existing real-device matrix compares selected storage backends, adjacent-read readahead, file-handle reuse, recheck/hash work, and HDD elevator scheduling. Its temporary files are scoped to a unique directory on the selected mount."
  echo
  echo "- Storage report: $STORAGE_REPORT"
  echo
  echo "System-wide device sample before the benchmark (includes unrelated workloads):"
  tail -40 "$IOSTAT_BEFORE"
} >"$OUT"

echo "==> running real-device storage matrix on $STORAGE_ROOT" >&2
if TNG_STORAGE_MATRIX_REPORT="$STORAGE_REPORT" \
   TNG_STORAGE_BENCH_DIR="$STORAGE_SCRATCH" \
   TNG_STORAGE_BENCH_BLOCKS="${TNG_BENCH_STORAGE_BLOCKS:-8192}" \
   TNG_STORAGE_BENCH_READS="${TNG_BENCH_STORAGE_READS:-10000}" \
   TNG_STORAGE_ELEVATOR_TRIALS="${TNG_BENCH_ELEVATOR_TRIALS:-3}" \
   nice -n 10 ionice -c 3 "$ROOT/scripts/storage_hardware_matrix.sh" "$STORAGE_SCRATCH" \
   >"$TMP_DIR/storage.log" 2>&1; then
  STORAGE_STATUS="PASS"
else
  STORAGE_STATUS="FAIL"
fi
{
  echo "- Matrix result: $STORAGE_STATUS"
  echo "- Matrix report exists: $([[ -s "$STORAGE_REPORT" ]] && echo yes || echo no)"
  echo "- Elevator/readahead/backend lines:"
  echo 'text output follows'
  if [[ -f "$STORAGE_REPORT" ]]; then
    rg -n 'tng_storage_(backend_stream|readahead|file_pool|recheck|shuffled_baseline|elevator)|elevator wall-clock ratio|selected=' "$STORAGE_REPORT" | tail -80 || true
  else
    tail -80 "$TMP_DIR/storage.log" || true
  fi
} >>"$OUT"

cat >"$TMP_DIR/sample_process.py" <<'PY'
import csv, glob, os, sys, time
pid, output, phase_path, interval = int(sys.argv[1]), sys.argv[2], sys.argv[3], float(sys.argv[4])
hz = os.sysconf("SC_CLK_TCK")
page = os.sysconf("SC_PAGE_SIZE")
previous_ticks = None
previous_at = None
with open(output, "w", newline="", buffering=1) as handle:
    writer = csv.writer(handle)
    writer.writerow(["utc", "phase", "cpu_pct_cores", "cpu_seconds", "rss_mib", "read_bytes", "write_bytes"])
    while True:
        try:
            stat = open(f"/proc/{pid}/stat").read()
            tail = stat[stat.rfind(")") + 2:].split()
            total_ticks = int(tail[11]) + int(tail[12])
            rss_pages = int(tail[21])
            io = {}
            for line in open(f"/proc/{pid}/io"):
                key, value = line.split(":", 1)
                io[key] = int(value.strip())
            thread_ticks = 0
            for thread_stat in glob.glob(f"/proc/{pid}/task/*/stat"):
                try:
                    thread = open(thread_stat).read()
                    fields = thread[thread.rfind(")") + 2:].split()
                    thread_ticks += int(fields[11]) + int(fields[12])
                except (OSError, ValueError, IndexError):
                    pass
            if thread_ticks:
                total_ticks = thread_ticks
            now = time.monotonic()
            cpu_pct = 0.0 if previous_ticks is None else max(0.0, (total_ticks - previous_ticks) / hz / max(now - previous_at, 0.001) * 100.0)
            previous_ticks, previous_at = total_ticks, now
            phase = open(phase_path).read().strip() or "unknown"
            writer.writerow([time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), phase, f"{cpu_pct:.2f}", f"{total_ticks / hz:.3f}", f"{rss_pages * page / 1048576:.2f}", io.get("read_bytes", 0), io.get("write_bytes", 0)])
            time.sleep(interval)
        except (OSError, ValueError, IndexError):
            break
PY

printf 'idle\n' >"$TMP_DIR/phase"
TORRENTNGD_CONFIG="$CONFIG" RUST_LOG=warn nice -n 10 ionice -c 3 "$BIN" >"$TMP_DIR/daemon.log" 2>&1 &
PID=$!
ready=0
for _ in $(seq 1 90); do
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "torrentngd exited before readiness" >&2
    tail -80 "$TMP_DIR/daemon.log" >&2 || true
    break
  fi
  if curl --max-time 2 -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
if [[ "$ready" != 1 ]]; then
  echo "torrentngd did not become ready on loopback" >&2
  exit 1
fi
echo "- Daemon effective nice: $(ps -o ni= -p "$PID" | awk '{print $1}')" >>"$OUT"

printf 'idle\n' >"$TMP_DIR/phase"
python3 "$TMP_DIR/sample_process.py" "$PID" "$SAMPLES" "$TMP_DIR/phase" "$SAMPLE_SECS" &
SAMPLER_PID=$!
curl --max-time 10 -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/metrics" >"$METRICS_LOG" 2>/dev/null || true
sleep 10

echo "==> adding official Linux ISO torrent" >&2
add_code="$(curl --max-time 30 -sS -o "$TMP_DIR/add-response.txt" -w '%{http_code}' \
  -H "Authorization: Bearer $TOKEN" \
  -F "urls=$TORRENT_URL" \
  -F "savepath=$DATA_DIR" \
  -F "category=benchmark-$STAMP" \
  -F 'stopped=false' \
  "http://127.0.0.1:$API_PORT/api/qb/v2/torrents/add" || true)"
if [[ "$add_code" != 200 ]]; then
  TRANSFER_STATUS="ADD_FAILED"
  echo "torrent add failed: HTTP $add_code $(cat "$TMP_DIR/add-response.txt" 2>/dev/null || true)" >&2
else
  printf 'transfer\n' >"$TMP_DIR/phase"
  transfer_start="$(date +%s)"
  transfer_deadline=$((transfer_start + TIMEOUT_SECS))
  printf 'utc\tgovernor_used_bytes\tgovernor_cap_bytes\tframe_in_use_bytes\tframe_cap_bytes\n' >"$METRICS_POLL"
  while (( $(date +%s) < transfer_deadline )); do
    info="$(curl --max-time 15 -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/api/qb/v2/torrents/info" 2>/dev/null || echo '[]')"
    metric_values="$(curl --max-time 15 -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/metrics" 2>/dev/null | awk '
      $1 == "torrentng_memory_used_bytes" { used=$2 }
      $1 == "torrentng_memory_cap_bytes" { cap=$2 }
      $1 == "torrentng_storage_frame_bytes_in_use" { frame=$2 }
      $1 == "torrentng_storage_frame_bytes_cap" { frame_cap=$2 }
      END { printf "%s\t%s\t%s\t%s", used, cap, frame, frame_cap }
    ' || true)"
    printf '%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$metric_values" >>"$METRICS_POLL"
    row="$(jq -c --arg name "$TORRENT_NAME" '[.[] | select(.name == $name)] | first // empty' <<<"$info" 2>/dev/null || true)"
    if [[ -n "$row" ]]; then
      jq -c --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg elapsed "$(( $(date +%s) - transfer_start ))" \
        '. + {sample_utc:$ts, elapsed_seconds:($elapsed|tonumber)} | {sample_utc,elapsed_seconds,progress,downloaded,dlspeed,upspeed,num_seeds,num_leechs,state}' \
        <<<"$row" >>"$POLL_LOG"
      if jq -e '.progress >= 1 and .amount_left == 0' <<<"$row" >/dev/null 2>&1; then
        TRANSFER_STATUS="PASS"
        break
      fi
    fi
    sleep 5
  done
  if [[ "$TRANSFER_STATUS" == NOT_RUN ]]; then
    TRANSFER_STATUS="TIMEOUT"
  fi
  if [[ -s "$POLL_LOG" ]]; then
    final_row="$(tail -1 "$POLL_LOG")"
    downloaded="$(jq -r '.downloaded // 0' <<<"$final_row")"
    elapsed="$(jq -r '.elapsed_seconds // 0' <<<"$final_row")"
    avg_rate="$(awk -v n="$downloaded" -v t="$elapsed" 'BEGIN {if (t > 0) printf "%.2f", n/t/1048576; else print "0"}')"
  else
    downloaded=0
    elapsed=0
    avg_rate=0
    final_row='{}'
  fi
  {
    echo
    echo "## Public ISO transfer"
    echo
    echo "- Status: $TRANSFER_STATUS"
    echo "- Torrent: $TORRENT_URL"
    echo "- Filename: $TORRENT_NAME"
    echo "- Last API sample: $final_row"
    echo "- Bytes reported downloaded: $downloaded"
    echo "- Elapsed transfer seconds: $elapsed"
    echo "- Mean API download rate: $avg_rate MiB/s"
    echo "- Download cap: $DOWNLOAD_LIMIT bytes/sec"
    echo "- Poll samples: $(wc -l <"$POLL_LOG" 2>/dev/null || echo 0)"
  } >>"$OUT"
  if [[ "$TRANSFER_STATUS" == PASS ]]; then
    sums_file="$TMP_DIR/SHA256SUMS"
    if curl --max-time 30 -fsSL "$SHA256SUMS_URL" -o "$sums_file"; then
      expected="$(awk -v name="$TORRENT_NAME" '$2 == name || $2 == "*" name {print $1; exit}' "$sums_file")"
      actual="$(sha256sum "$DATA_DIR/$TORRENT_NAME" | awk '{print $1}')"
      if [[ -n "$expected" && "$actual" == "$expected" ]]; then
        HASH_STATUS="PASS"
      else
        HASH_STATUS="FAIL"
      fi
      {
        echo "- Official checksum status: $HASH_STATUS"
        echo "- SHA-256 expected: ${expected:-not-found}"
        echo "- SHA-256 actual: $actual"
      } >>"$OUT"
    else
      HASH_STATUS="UNAVAILABLE"
      echo "- Official checksum status: UNAVAILABLE (SHA256SUMS fetch failed)" >>"$OUT"
    fi
  fi
fi

printf 'after-transfer\n' >"$TMP_DIR/phase"
sleep 4
curl --max-time 10 -fsS -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$API_PORT/metrics" >"$TMP_DIR/metrics-after.txt" 2>/dev/null || true
kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
wait "$SAMPLER_PID" 2>/dev/null || true
SAMPLER_PID=""

{
  echo
  echo "## Memory-governor and frame-pool metrics"
  echo
  echo "These counters describe managed memory classes and storage frames; process RSS is reported separately below."
  echo
  grep -E '^torrentng_(memory_|storage_frame_bytes_)' "$METRICS_LOG" "$TMP_DIR/metrics-after.txt" || true
  if [[ -s "$METRICS_POLL" ]]; then
    awk -F '\t' 'NR==1 {next} {if ($2+0>max_used) max_used=$2; if ($3+0>max_cap) max_cap=$3; if ($4+0>max_frame) max_frame=$4; if ($5+0>max_frame_cap) max_frame_cap=$5; n++} END {printf "Peak sampled governor bytes/cap: %d/%d; frame bytes/cap: %d/%d; samples=%d\n", max_used,max_cap,max_frame,max_frame_cap,n}' "$METRICS_POLL"
  fi
} >>"$OUT"

python3 - "$SAMPLES" "$OUT" "$SAMPLE_SECS" <<'PY'
import csv, statistics, sys
rows = list(csv.DictReader(open(sys.argv[1])))
out = open(sys.argv[2], "a")
out.write("\n## Process resource samples\n\n")
out.write("Samples came from /proc for the isolated torrentngd PID; CPU is percent of one CPU core and RSS is resident MiB. Process I/O counters are kernel-accounted block bytes, not network bytes.\n\n")
out.write("| Phase | Samples | Mean CPU (% of one core) | Peak CPU (% of one core) | Peak RSS (MiB) |\n|---|---:|---:|---:|---:|\n")
for phase in ("idle", "transfer", "after-transfer"):
    group = [r for r in rows if r["phase"] == phase]
    if not group:
        continue
    cpu = [float(r["cpu_pct_cores"]) for r in group]
    rss = [float(r["rss_mib"]) for r in group]
    out.write(f"| {phase} | {len(group)} | {statistics.mean(cpu):.2f} | {max(cpu):.2f} | {max(rss):.2f} |\n")
if rows:
    first, last = rows[0], rows[-1]
    read_bytes = max(0, int(last["read_bytes"]) - int(first["read_bytes"]))
    write_bytes = max(0, int(last["write_bytes"]) - int(first["write_bytes"]))
    cpu_seconds = max(0.0, float(last["cpu_seconds"]) - float(first["cpu_seconds"]))
    out.write(f"\n- Process CPU time: {cpu_seconds:.2f} seconds\n")
    out.write(f"- Process block read bytes: {read_bytes}\n")
    out.write(f"- Process block write bytes: {write_bytes}\n")
    out.write(f"- Raw samples: {len(rows)}; interval: {sys.argv[3]} seconds\n")
else:
    out.write("\nNo process samples were collected.\n")
out.close()
PY

{
  echo
  echo "## Host context after run"
  echo
  echo "- Host load at end: $(awk '{print $1 ", " $2 ", " $3}' /proc/loadavg)"
  echo "- Available RAM at end: $(awk '/MemAvailable/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
  echo "- iostat sample (system-wide; includes unrelated workloads):"
  echo "System-wide device data follows:"
  iostat -dx 1 2 | tail -40
  echo "- Storage result: $STORAGE_STATUS"
  echo "- Torrent completion result: $TRANSFER_STATUS"
  echo "- Official ISO checksum result: $HASH_STATUS"
  echo "- The daemon ran with its API bound to loopback and exited before cleanup."
} >>"$OUT"

if [[ "$STORAGE_STATUS" != PASS || "$TRANSFER_STATUS" != PASS || "$HASH_STATUS" != PASS ]]; then
  exit 1
fi
