#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/curl_policy.sh
source "$ROOT/scripts/curl_policy.sh"
# shellcheck source=scripts/transfer_churn_report.sh
source "$ROOT/scripts/transfer_churn_report.sh"
ENV_FILE="${CERT_ENV_FILE:-$ROOT/deploy/certification/.env}"
COMPOSE_PROJECT="${CERT_COMPOSE_PROJECT:-certification}"
COMPOSE_NETWORK="${CERT_COMPOSE_NETWORK:-${COMPOSE_PROJECT}_default}"
DOWNLOADS_VOLUME="${CERT_DOWNLOADS_VOLUME:-${COMPOSE_PROJECT}_downloads}"
OUT="${1:-$ROOT/certification/reports/transfer-churn-$(date -u +%Y%m%dT%H%M%SZ)-$$.md}"

ENV_TNG_HOST_URL="${TNG_HOST_URL:-}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

TNG_HOST_URL="${ENV_TNG_HOST_URL:-${TNG_HOST_URL:-http://localhost:${TNG_HOST_PORT:-18080}}}"
TNG_API_TOKEN="${TNG_API_TOKEN:-local-cert-api-token-20260904}"
TNG_CONTAINER="${TNG_CONTAINER:-certification-torrentng-1}"
CHURN_CYCLES="${TRANSFER_CHURN_CYCLES:-5}"
FIXTURE_BYTES="${TRANSFER_CHURN_FIXTURE_BYTES:-16777216}"
TRANSFER_TIMEOUT_SECS="${TRANSFER_CHURN_TIMEOUT_SECS:-180}"
READY_TIMEOUT_SECS="${TRANSFER_CHURN_READY_TIMEOUT_SECS:-60}"
MAX_RSS_MB="${TRANSFER_CHURN_MAX_RSS_MB:-500}"
PUBLIC_TORRENT_URL="${PUBLIC_TORRENT_URL:-https://mirror.arizona.edu/debian-cd/current/amd64/bt-cd/debian-13.4.0-amd64-netinst.iso.torrent}"
PUBLIC_CYCLES="${TRANSFER_CHURN_PUBLIC_CYCLES:-0}"
MIN_PUBLIC_DOWNLOADED_BYTES="${TRANSFER_CHURN_PUBLIC_MIN_DOWNLOADED_BYTES:-16384}"
DOCKER_COMMAND_TIMEOUT_SECS="${TRANSFER_CHURN_DOCKER_COMMAND_TIMEOUT_SECS:-90}"
FIXTURE_ID_SEED="${TRANSFER_CHURN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"

if [[ ! "$FIXTURE_ID_SEED" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$ || "$FIXTURE_ID_SEED" == *..* ]]; then
  echo "TRANSFER_CHURN_ID must be 1-32 safe filename characters without '..'" >&2
  exit 2
fi
FIXTURE_ID="$FIXTURE_ID_SEED-$$"

normalize_uint() {
  local name="$1" value="$2" minimum="$3" maximum="$4" normalized
  if [[ ! "$value" =~ ^[0-9]{1,8}$ ]]; then
    echo "$name must be an integer between $minimum and $maximum" >&2
    exit 2
  fi
  normalized=$((10#$value))
  if (( normalized < minimum || normalized > maximum )); then
    echo "$name must be an integer between $minimum and $maximum" >&2
    exit 2
  fi
  printf '%s' "$normalized"
}

CHURN_CYCLES="$(normalize_uint TRANSFER_CHURN_CYCLES "$CHURN_CYCLES" 1 100)"
FIXTURE_BYTES="$(normalize_uint TRANSFER_CHURN_FIXTURE_BYTES "$FIXTURE_BYTES" 1 67108864)"
TRANSFER_TIMEOUT_SECS="$(normalize_uint TRANSFER_CHURN_TIMEOUT_SECS "$TRANSFER_TIMEOUT_SECS" 1 3600)"
READY_TIMEOUT_SECS="$(normalize_uint TRANSFER_CHURN_READY_TIMEOUT_SECS "$READY_TIMEOUT_SECS" 1 300)"
MAX_RSS_MB="$(normalize_uint TRANSFER_CHURN_MAX_RSS_MB "$MAX_RSS_MB" 1 65536)"
PUBLIC_CYCLES="$(normalize_uint TRANSFER_CHURN_PUBLIC_CYCLES "$PUBLIC_CYCLES" 0 20)"
MIN_PUBLIC_DOWNLOADED_BYTES="$(normalize_uint TRANSFER_CHURN_PUBLIC_MIN_DOWNLOADED_BYTES "$MIN_PUBLIC_DOWNLOADED_BYTES" 1 67108864)"
DOCKER_COMMAND_TIMEOUT_SECS="$(normalize_uint TRANSFER_CHURN_DOCKER_COMMAND_TIMEOUT_SECS "$DOCKER_COMMAND_TIMEOUT_SECS" 1 300)"

command -v timeout >/dev/null || {
  echo "timeout is required for bounded Docker readiness probes" >&2
  exit 2
}

CHURN_DOWNLOAD_DIR="transfer-churn-$FIXTURE_ID"
CATEGORY="cert-transfer-churn-$FIXTURE_ID"
TRACKER_NAME="tng-churn-tracker-$FIXTURE_ID"
FILESERVER_NAME="tng-churn-files-$FIXTURE_ID"
COOKIE_JAR=""
BODY=""
AUTH_BODY_FILE=""
DOCKER_RESOURCES_ATTEMPTED="0"
FINALIZED="0"

mkdir -p "$(dirname "$OUT")"

mapped="$(timeout --signal=TERM --kill-after=1s 5s \
  docker port "$TNG_CONTAINER" 8080/tcp 2>/dev/null | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p' | head -1 || true)"
if [[ -n "$mapped" && "$TNG_HOST_URL" == http://localhost:* ]]; then
  TNG_HOST_URL="http://localhost:$mapped"
fi
python3 "$ROOT/scripts/protected_target.py" "$TNG_HOST_URL"
COOKIE_JAR="$(mktemp "${TMPDIR:-/tmp}/tng-churn-cookies.XXXXXX")"
BODY="$(mktemp "${TMPDIR:-/tmp}/tng-churn-body.XXXXXX")"
AUTH_BODY_FILE="$(mktemp "${TMPDIR:-/tmp}/tng-churn-auth.XXXXXX")"

status="PASS"
REPORT_WRITE_FAILED="0"
FAILURE_DIAGNOSTICS_CAPTURED="0"
TNG_AUTHENTICATED="0"
seeders=()
cleanup_failures=()

mark() {
  local name="$1"
  local result="$2"
  local detail="$3" record
  detail="${detail//$'\n'/ }"
  detail="${detail//|/\\|}"
  printf -v record '| %s | %s | %s |\n' "$name" "$result" "$detail"
  report_append "$record" || return 1
  [[ "$result" == "FAIL" ]] && status="FAIL"
  return 0
}

capture_failure_diagnostics() {
  [[ "$status" == "FAIL" && "$FAILURE_DIAGNOSTICS_CAPTURED" == "0" ]] || return 0
  FAILURE_DIAGNOSTICS_CAPTURED="1"
  local diagnostics container
  diagnostics="$(
    echo
    echo "## Failure Diagnostics"
    if [[ -s "$COOKIE_JAR" ]]; then
      local torrent_state
      torrent_state="$(curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 -b "$COOKIE_JAR" \
        "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" 2>/dev/null \
        | jq -c '[.[] | {name,state,progress,downloaded,dlspeed,num_seeds,num_leechs,category}]' 2>/dev/null || true)"
      echo "- Torrent state: ${torrent_state:-unavailable}"
    fi
    echo
    echo "### TorrentNG daemon log tail"
    timeout --signal=TERM --kill-after=1s 5s docker logs --tail 100 "$TNG_CONTAINER" 2>&1 || true
    for container in "$TRACKER_NAME" "$FILESERVER_NAME" "${seeders[@]}"; do
      [[ -n "$container" ]] || continue
      if timeout --signal=TERM --kill-after=1s 5s docker inspect "$container" >/dev/null 2>&1; then
        echo
        echo "### $container"
        timeout --signal=TERM --kill-after=1s 5s docker inspect --format 'state={{.State.Status}} exit={{.State.ExitCode}} error={{.State.Error}}' "$container" 2>/dev/null || true
        timeout --signal=TERM --kill-after=1s 5s docker logs --tail 80 "$container" 2>&1 || true
      fi
    done
  )"
  report_append "$diagnostics"$'\n'
}

cleanup_container() {
  local name="$1" output output_lower inspect_status
  output="$(timeout --signal=TERM --kill-after=1s 10s docker inspect "$name" 2>&1)"
  inspect_status=$?
  if (( inspect_status == 0 )); then
    timeout --signal=TERM --kill-after=1s 10s docker rm -f "$name" >/dev/null 2>&1 || return 1
    output="$(timeout --signal=TERM --kill-after=1s 10s docker inspect "$name" 2>&1)"
    inspect_status=$?
  fi
  output_lower="${output,,}"
  (( inspect_status == 1 )) && [[ "$output_lower" == *"no such object"* || "$output_lower" == *"no such container"* ]]
}

remove_volume_path() {
  local path="$1" suffix
  if [[ "$path" != "/downloads/$CHURN_DOWNLOAD_DIR" ]]; then
    suffix="${path#"/downloads/$CHURN_DOWNLOAD_DIR/"}"
    [[ "$suffix" =~ ^([0-9]+|public/[0-9]+)$ ]] || return 2
  fi
  # These single-quoted commands are interpreted inside the Alpine container.
  # shellcheck disable=SC2016
  timeout --signal=TERM --kill-after=1s 30s \
    docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 sh -c 'rm -rf -- "$1"' sh "$path" >/dev/null 2>&1 || return 1
  # shellcheck disable=SC2016
  if timeout --signal=TERM --kill-after=1s 30s \
    docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 sh -c 'test ! -e "$1"' sh "$path" >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

# shellcheck disable=SC2329 # Called by cleanup, which is installed as an EXIT trap.
finalize_report() {
  [[ "$FINALIZED" == "1" ]] && return 0
  FINALIZED="1"
  report_append $'\n## Cleanup\n\n| Check | Result | Detail |\n|---|---|---|\n' || status="FAIL"
  if (( ${#cleanup_failures[@]} == 0 )); then
    mark "task resources removed" "PASS" "category, named containers, and run-scoped volume directory verified clean" || status="FAIL"
  else
    local detail
    detail="$(IFS=', '; echo "${cleanup_failures[*]}")"
    mark "task resource cleanup" "FAIL" "$detail" || status="FAIL"
  fi
  [[ "$REPORT_WRITE_FAILED" == "1" ]] && status="FAIL"
  report_append $'\nOverall status: '"$status"$'\n' || status="FAIL"
  printf '%s\n' "$OUT"
}

# shellcheck disable=SC2329 # Called by the EXIT/INT/TERM/HUP traps.
cleanup() {
  local original_exit=$? name
  trap - EXIT INT TERM HUP
  set +e
  if (( original_exit != 0 )); then
    status="FAIL"
  fi
  capture_failure_diagnostics
  if [[ "$TNG_AUTHENTICATED" == "1" ]] && ! delete_category_torrents; then
    cleanup_failures+=("could not confirm removal of category $CATEGORY")
  fi
  if [[ "$DOCKER_RESOURCES_ATTEMPTED" == "1" ]]; then
    if ! timeout --signal=TERM --kill-after=1s 5s docker info >/dev/null 2>&1; then
      cleanup_failures+=("Docker daemon unavailable; task containers and volume directory could not be verified")
    else
      for name in "${seeders[@]}" "$FILESERVER_NAME" "$TRACKER_NAME"; do
        [[ -n "$name" ]] || continue
        if ! cleanup_container "$name"; then
          cleanup_failures+=("container $name remains or could not be verified absent")
        fi
      done
      if ! remove_volume_path "/downloads/$CHURN_DOWNLOAD_DIR"; then
        cleanup_failures+=("volume path /downloads/$CHURN_DOWNLOAD_DIR remains or could not be verified absent")
      fi
    fi
  fi
  for name in "$COOKIE_JAR" "$BODY" "$AUTH_BODY_FILE"; do
    [[ -n "$name" ]] || continue
    if ! rm -f -- "$name" || [[ -e "$name" ]]; then
      cleanup_failures+=("temporary file $name remains")
    fi
  done
  if (( original_exit != 0 )) && [[ "$status" == "PASS" ]]; then
    status="FAIL"
  fi
  finalize_report
  if [[ "$status" == "FAIL" || "$REPORT_WRITE_FAILED" == "1" || $original_exit -ne 0 ]]; then
    exit 1
  fi
  exit 0
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
tng_write_qbit_login_body "$TNG_API_TOKEN" "$AUTH_BODY_FILE"

http_code() {
  local url="$1"
  shift || true
  curl -q -sS --noproxy "*" --connect-timeout 3 --max-time 10 \
    -o "$BODY" -w '%{http_code}' "$@" "$url" || true
}

bounded_command() {
  timeout --signal=TERM --kill-after=1s "${DOCKER_COMMAND_TIMEOUT_SECS}s" "$@"
}

bounded_for_deadline() {
  local deadline="$1" remaining
  shift
  remaining=$((deadline - SECONDS))
  (( remaining > 0 )) || return 124
  timeout --signal=TERM --kill-after=1s "${remaining}s" "$@"
}

wait_for_container_dns() {
  local hostname="$1"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    if bounded_for_deadline "$deadline" docker exec "$TNG_CONTAINER" getent hosts "$hostname" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

container_http_code() {
  local url="$1"
  local deadline="$2"
  bounded_for_deadline "$deadline" docker exec "$TNG_CONTAINER" curl -q -sS --noproxy "*" \
    --connect-timeout 1 --max-time 2 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || printf '000'
  return 0
}

wait_for_container_http_status() {
  local url="$1"
  local expected="$2"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  local code
  while (( SECONDS < deadline )); do
    code="$(container_http_code "$url" "$deadline")"
    if [[ "$code" == "$expected" ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_container_http_response() {
  local url="$1"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  local code
  while (( SECONDS < deadline )); do
    code="$(container_http_code "$url" "$deadline")"
    if [[ "$code" =~ ^[1-5][0-9]{2}$ ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_seeder_listener() {
  local container="$1"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  local state logs
  while (( SECONDS < deadline )); do
    state="$(bounded_for_deadline "$deadline" docker inspect --format '{{.State.Status}}' "$container" 2>/dev/null || true)"
    if [[ "$state" == "running" ]]; then
      logs="$(bounded_for_deadline "$deadline" docker logs --tail 100 "$container" 2>&1 || true)"
      if [[ "$logs" == *"Listening to incoming peer connections"* ]]; then
        return 0
      fi
    fi
    sleep 1
  done
  return 1
}

torrent_info_hash() {
  local torrent_path="$1"
  bounded_command docker exec "$TNG_CONTAINER" cat "$torrent_path" | python3 -c '
import hashlib, sys
data = sys.stdin.buffer.read()

def parse_string(offset):
    colon = data.index(b":", offset)
    length = int(data[offset:colon])
    start = colon + 1
    return data[start:start + length], start + length

def skip_value(offset):
    marker = data[offset:offset + 1]
    if marker == b"i":
        return data.index(b"e", offset + 1) + 1
    if marker in (b"l", b"d"):
        is_dict = marker == b"d"
        offset += 1
        while data[offset:offset + 1] != b"e":
            offset = skip_value(offset)
            if is_dict:
                offset = skip_value(offset)
        return offset + 1
    _, offset = parse_string(offset)
    return offset

if data[:1] != b"d":
    raise SystemExit("torrent metainfo is not a dictionary")
offset = 1
while data[offset:offset + 1] != b"e":
    key, offset = parse_string(offset)
    start = offset
    offset = skip_value(offset)
    if key == b"info":
        print(hashlib.sha1(data[start:offset]).hexdigest())
        break
else:
    raise SystemExit("torrent metainfo has no info dictionary")
'
}

tracker_complete_count() {
  local info_hash="$1"
  local deadline="$2"
  local encoded_hash
  encoded_hash="$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote_from_bytes(bytes.fromhex(sys.argv[1]), safe=""))' "$info_hash")"
  bounded_for_deadline "$deadline" docker exec "$TNG_CONTAINER" curl -q -sS --noproxy "*" \
    --connect-timeout 1 --max-time 2 \
    "http://$TRACKER_NAME:6969/scrape?info_hash=$encoded_hash" 2>/dev/null \
    | python3 -c '
import sys
data = sys.stdin.buffer.read()

def decode(offset):
    marker = data[offset:offset + 1]
    if marker == b"i":
        end = data.index(b"e", offset + 1)
        return int(data[offset + 1:end]), end + 1
    if marker in (b"l", b"d"):
        is_dict = marker == b"d"
        result = {} if is_dict else []
        offset += 1
        while data[offset:offset + 1] != b"e":
            key, offset = decode(offset)
            if is_dict:
                value, offset = decode(offset)
                result[key] = value
            else:
                result.append(key)
        return result, offset + 1
    colon = data.index(b":", offset)
    length = int(data[offset:colon])
    start = colon + 1
    return data[start:start + length], start + length

try:
    response, _ = decode(0)
    info_hash = bytes.fromhex(sys.argv[1])
    print(response.get(b"files", {}).get(info_hash, {}).get(b"complete", 0))
except (IndexError, ValueError, TypeError):
    print(0)
' "$info_hash" 2>/dev/null || echo 0
}

wait_for_tracker_seed() {
  local info_hash="$1"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  local count
  while (( SECONDS < deadline )); do
    count="$(tracker_complete_count "$info_hash" "$deadline")"
    if [[ "$count" =~ ^[0-9]+$ ]] && (( count >= 1 )); then
      return 0
    fi
    sleep 1
  done
  return 1
}

runtime_owner() {
  local owner
  # The single-quoted program is interpreted by the container's shell.
  # shellcheck disable=SC2016
  owner="$(bounded_command docker exec "$TNG_CONTAINER" sh -lc '
for status in /proc/[0-9]*/status; do
  [ -r "$status" ] || continue
  name="$(awk "/^Name:/ {print \$2; exit}" "$status")"
  [ "$name" = torrentngd ] || continue
  awk "/^Uid:/ {uid=\$2} /^Gid:/ {gid=\$2} END {if (uid != \"\" && gid != \"\") print uid \":\" gid}" "$status"
  exit
done
' 2>/dev/null || true)"
  [[ "$owner" =~ ^[1-9][0-9]*:[1-9][0-9]*$ ]] || return 1
  printf '%s\n' "$owner"
}

rss_mb() {
  # The single-quoted program is interpreted by the container's shell.
  # shellcheck disable=SC2016
  bounded_command docker exec "$TNG_CONTAINER" sh -lc '
for status in /proc/[0-9]*/status; do
  [ -r "$status" ] || continue
  name="$(awk "/^Name:/ {print \$2; exit}" "$status")"
  [ "$name" = torrentngd ] || continue
  awk "/^VmHWM:/ {printf \"%.1f\", \$2 / 1024; found=1} END {if (!found) exit 1}" "$status"
  exit
done
exit 1
' 2>/dev/null
}

delete_category_torrents() {
  local code hashes deadline
  code="$(curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 -b "$COOKIE_JAR" \
    -o "$BODY" -w '%{http_code}' \
    "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY")" || return 1
  [[ "$code" == "200" ]] || return 1
  jq -e 'type == "array"' "$BODY" >/dev/null 2>&1 || return 1
  hashes="$(python3 "$ROOT/scripts/transfer_churn_validation.py" category-hashes < "$BODY")" || return 1
  [[ "$hashes" != "EMPTY" ]] || return 0

  code="$(curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 -b "$COOKIE_JAR" \
    -X POST -d "hashes=$hashes" -d "deleteFiles=true" \
    -o "$BODY" -w '%{http_code}' "$TNG_HOST_URL/api/qb/v2/torrents/delete")" || return 1
  [[ "$code" == "200" ]] || return 1

  deadline=$((SECONDS + 15))
  while (( SECONDS < deadline )); do
    code="$(curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 -b "$COOKIE_JAR" \
      -o "$BODY" -w '%{http_code}' \
      "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY")" || return 1
    if [[ "$code" == "200" ]] && jq -e 'type == "array" and length == 0' "$BODY" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

read_disk_bytes() {
  local path="$1" deadline="$2" result
  # The single-quoted command is interpreted inside the Alpine container.
  # shellcheck disable=SC2016
  result="$(bounded_for_deadline "$deadline" docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 \
    sh -c 'if [ -f "$1" ]; then wc -c < "$1"; else printf 0; fi' sh "$path" 2>/dev/null)" || return 1
  result="$(tr -d '[:space:]' <<<"$result")"
  [[ "$result" =~ ^[0-9]+$ ]] || return 1
  printf '%s\n' "$result"
}

read_file_sha256() {
  local path="$1" deadline="$2" digest
  digest="$(bounded_for_deadline "$deadline" docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 \
    sha256sum "$path" 2>/dev/null | awk '{print $1}')" || return 1
  [[ "$digest" =~ ^[[:xdigit:]]{64}$ ]] || return 1
  printf '%s\n' "$digest"
}

wait_for_completion() {
  local name="$1"
  local path="$2"
  local bytes="$3"
  local expected_sha256="$4"
  local deadline=$((SECONDS + TRANSFER_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    local row observed actual_sha256
    row="$(bounded_for_deadline "$deadline" curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 \
      -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" \
      | jq -c --arg name "$name" '.[] | select(.name==$name)' | head -1 || true)"
    if [[ -n "$row" ]] && jq -e --argjson bytes "$bytes" \
      '(.progress >= 1) and (.downloaded >= $bytes)' <<<"$row" >/dev/null 2>&1; then
      observed="$(read_disk_bytes "$path" "$deadline")" || observed=""
      if [[ "$observed" == "$bytes" ]]; then
        actual_sha256="$(read_file_sha256 "$path" "$deadline")" || actual_sha256=""
        if [[ -n "$actual_sha256" ]] && printf '%s\n' "$row" | \
          python3 "$ROOT/scripts/transfer_churn_validation.py" complete \
            --expected-bytes "$bytes" --disk-bytes "$observed" \
            --expected-sha256 "$expected_sha256" --actual-sha256 "$actual_sha256"; then
          printf '%s' "$row"
          return 0
        fi
      fi
    fi
    sleep 1
  done
  return 1
}

wait_for_public_transfer() {
  local deadline=$((SECONDS + TRANSFER_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    local row
    row="$(bounded_for_deadline "$deadline" curl -q -sS --noproxy "*" --connect-timeout 2 --max-time 5 \
      -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" \
      | jq -c '.[0] // empty' || true)"
    if [[ -n "$row" ]] && printf '%s\n' "$row" | \
      python3 "$ROOT/scripts/transfer_churn_validation.py" public \
        --minimum-downloaded-bytes "$MIN_PUBLIC_DOWNLOADED_BYTES"; then
      printf '%s' "$row"
      return 0
    fi
    sleep 2
  done
  return 1
}

report_header="# TorrentNG Transfer Churn Soak

- Date UTC: $(date -u +%Y-%m-%dT%H:%M:%SZ)
- Run ID: $FIXTURE_ID
- TorrentNG URL: $TNG_HOST_URL
- Docker network: $COMPOSE_NETWORK
- Downloads volume: $DOWNLOADS_VOLUME
- Local cycles: $CHURN_CYCLES
- Fixture bytes per cycle: $FIXTURE_BYTES
- Public Linux torrent cycles: $PUBLIC_CYCLES"
if [[ "$PUBLIC_CYCLES" != "0" ]]; then
  report_header+=$'\n- Public torrent URL: '"$PUBLIC_TORRENT_URL"
  report_header+=$'\n- Minimum public payload bytes: '"$MIN_PUBLIC_DOWNLOADED_BYTES"
fi
report_header+=$'\n- Transfer timeout seconds: '"$TRANSFER_TIMEOUT_SECS"
report_header+=$'\n- Readiness timeout seconds: '"$READY_TIMEOUT_SECS"
report_header+=$'\n- Docker command timeout seconds: '"$DOCKER_COMMAND_TIMEOUT_SECS"
report_header+=$'\n- Max RSS MB: '"$MAX_RSS_MB"
report_header+=$'\n\n## Checks\n\n| Check | Result | Detail |\n|---|---|---|\n'
if ! report_create "$report_header"; then
  exit 1
fi

code="$(http_code "$TNG_HOST_URL/api/qb/v2/auth/login" -X POST --data-binary "@$AUTH_BODY_FILE" -c "$COOKIE_JAR")"
if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]]; then
  TNG_AUTHENTICATED="1"
  mark "qBit auth" "PASS" "session cookie accepted"
else
  mark "qBit auth" "FAIL" "HTTP $code body=$(tr '\n' ' ' <"$BODY")"
  exit 1
fi

if ! delete_category_torrents; then
  mark "initial category cleanup" "FAIL" "could not verify the run-scoped category is empty"
  exit 1
fi

DOCKER_RESOURCES_ATTEMPTED="1"
bounded_command docker run -d --label "com.torrentng.transfer-churn.run=$FIXTURE_ID" \
  --name "$TRACKER_NAME" --network "$COMPOSE_NETWORK" alpine:3.20 \
  sh -lc 'apk add --no-cache opentracker >/dev/null && exec opentracker -i 0.0.0.0 -p 6969 -P 6969' >/dev/null
if wait_for_container_dns "$TRACKER_NAME" \
  && wait_for_container_http_response "http://$TRACKER_NAME:6969/announce"; then
  mark "local tracker" "PASS" "DNS and HTTP ready at http://$TRACKER_NAME:6969/announce"
else
  mark "local tracker" "FAIL" "DNS/HTTP readiness timed out after ${READY_TIMEOUT_SECS}s"
  exit 1
fi

APP_RUNTIME_OWNER="$(runtime_owner)" || {
  mark "app runtime identity" "FAIL" "could not read torrentngd UID/GID from the container"
  exit 1
}
mark "app runtime identity" "PASS" "torrentngd runs as $APP_RUNTIME_OWNER"

# The single-quoted command is interpreted inside the Alpine container.
# shellcheck disable=SC2016
bounded_command docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 sh -c 'mkdir -p -- "$1"' sh "/downloads/$CHURN_DOWNLOAD_DIR"

bounded_command docker run -d --label "com.torrentng.transfer-churn.run=$FIXTURE_ID" \
  --name "$FILESERVER_NAME" --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 \
  sh -lc "apk add --no-cache busybox-extras >/dev/null && exec httpd -f -p 8081 -h /downloads/$CHURN_DOWNLOAD_DIR" >/dev/null
if wait_for_container_dns "$FILESERVER_NAME" \
  && wait_for_container_http_response "http://$FILESERVER_NAME:8081/"; then
  mark "fixture file server" "PASS" "DNS and HTTP ready at http://$FILESERVER_NAME:8081/"
else
  mark "fixture file server" "FAIL" "DNS/HTTP readiness timed out after ${READY_TIMEOUT_SECS}s"
  exit 1
fi

report_append $'\n## Transfer Cycles\n\n| Cycle | Type | Seeder tracked | Add HTTP | Progress | Downloaded | Payload SHA-256 | RSS MB | Result |\n|---:|---|---|---:|---:|---:|---|---:|---|\n'

max_rss="0"
for cycle in $(seq 1 "$CHURN_CYCLES"); do
  name="tng-churn-$FIXTURE_ID-$cycle.bin"
  torrent="tng-churn-$FIXTURE_ID-$cycle.torrent"
  seeder="tng-churn-seeder-$FIXTURE_ID-$cycle"
  seeders+=("$seeder")

  bounded_command docker run --rm --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 sh -lc "
    set -e
    apk add --no-cache mktorrent >/dev/null
    mkdir -p /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed /downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech
    chown $APP_RUNTIME_OWNER /downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech
    dd if=/dev/urandom of=/downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed/$name bs=$FIXTURE_BYTES count=1 status=none
    mktorrent -a http://$TRACKER_NAME:6969/announce -o /downloads/$CHURN_DOWNLOAD_DIR/$cycle/$torrent /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed/$name >/dev/null
  "

  seed_path="/downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed/$name"
  expected_sha256="$(read_file_sha256 "$seed_path" "$((SECONDS + DOCKER_COMMAND_TIMEOUT_SECS))")" || {
    mark "local fixture source hash" "FAIL" "could not hash generated seeder payload"
    exit 1
  }

  bounded_command docker run -d --label "com.torrentng.transfer-churn.run=$FIXTURE_ID" \
    --name "$seeder" --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
    sh -lc "apk add --no-cache transmission-cli >/dev/null && exec transmission-cli -M -et -w /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed /downloads/$CHURN_DOWNLOAD_DIR/$cycle/$torrent" >/dev/null

  fixture_url="http://$FILESERVER_NAME:8081/$cycle/$torrent"
  torrent_path="/data/$CHURN_DOWNLOAD_DIR/$cycle/$torrent"
  if wait_for_seeder_listener "$seeder" \
    && wait_for_container_http_status "$fixture_url" 200 \
    && info_hash="$(torrent_info_hash "$torrent_path")" \
    && wait_for_tracker_seed "$info_hash"; then
    seeder_tracked="PASS"
    code="$(curl -q -sS --noproxy "*" --connect-timeout 3 --max-time 10 -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
      -F "urls=$fixture_url" \
      -F "savepath=/data/$CHURN_DOWNLOAD_DIR/$cycle/leech" \
      -F "category=$CATEGORY" \
      -F "stopped=false" \
      "$TNG_HOST_URL/api/qb/v2/torrents/add" || true)"
  else
    seeder_tracked="FAIL"
    code="000"
  fi

  progress="0"
  downloaded="0"
  payload_hash="NOT VERIFIED"
  result="FAIL"
  if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]] && \
    row="$(wait_for_completion "$name" "/downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech/$name" "$FIXTURE_BYTES" "$expected_sha256")"; then
    progress="$(jq -r '.progress' <<<"$row")"
    downloaded="$(jq -r '.downloaded' <<<"$row")"
    payload_hash="MATCH"
    result="PASS"
  fi

  rss="$(rss_mb)"
  awk -v a="$rss" -v b="$max_rss" 'BEGIN {exit !(a > b)}' && max_rss="$rss"
  printf -v cycle_record '| %s | local fixture | %s | %s | %s | %s | %s | %s | %s |\n' \
    "$cycle" "$seeder_tracked" "$code" "$progress" "$downloaded" "$payload_hash" "$rss" "$result"
  report_append "$cycle_record"
  if [[ "$result" != "PASS" ]]; then
    status="FAIL"
    capture_failure_diagnostics
  fi

  if ! delete_category_torrents; then
    mark "cycle $cycle cleanup" "FAIL" "could not confirm removal of the run-scoped torrent"
    exit 1
  fi
  if ! cleanup_container "$seeder"; then
    mark "cycle $cycle cleanup" "FAIL" "could not remove seeder container $seeder"
    exit 1
  fi
  if ! remove_volume_path "/downloads/$CHURN_DOWNLOAD_DIR/$cycle"; then
    mark "cycle $cycle cleanup" "FAIL" "could not remove the cycle payload directory"
    exit 1
  fi
  seeders=()
done

for cycle in $(seq 1 "$PUBLIC_CYCLES"); do
  code="$(curl -q -sS --noproxy "*" --connect-timeout 3 --max-time 10 -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
    -F "urls=$PUBLIC_TORRENT_URL" \
    -F "savepath=/data/$CHURN_DOWNLOAD_DIR/public/$cycle" \
    -F "category=$CATEGORY" \
    -F "stopped=false" \
    "$TNG_HOST_URL/api/qb/v2/torrents/add" || true)"
  progress="0"
  downloaded="0"
  result="FAIL"
  if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]] && row="$(wait_for_public_transfer)"; then
    progress="$(jq -r '.progress' <<<"$row")"
    downloaded="$(jq -r '.downloaded' <<<"$row")"
    result="PASS"
  fi
  rss="$(rss_mb)"
  awk -v a="$rss" -v b="$max_rss" 'BEGIN {exit !(a > b)}' && max_rss="$rss"
  printf -v cycle_record '| %s | public Linux torrent | N/A | %s | %s | %s | N/A | %s | %s |\n' \
    "$cycle" "$code" "$progress" "$downloaded" "$rss" "$result"
  report_append "$cycle_record"
  if [[ "$result" != "PASS" ]]; then
    status="FAIL"
    capture_failure_diagnostics
  fi
  if ! delete_category_torrents; then
    mark "public cycle $cycle cleanup" "FAIL" "could not confirm removal of the run-scoped torrent"
    exit 1
  fi
  if ! remove_volume_path "/downloads/$CHURN_DOWNLOAD_DIR/public/$cycle"; then
    mark "public cycle $cycle cleanup" "FAIL" "could not remove the cycle payload directory"
    exit 1
  fi
done

if awk -v rss="$max_rss" -v limit="$MAX_RSS_MB" 'BEGIN {exit !(rss <= limit)}'; then
  mark "memory ceiling" "PASS" "torrentngd process high-water RSS ${max_rss}MB <= ${MAX_RSS_MB}MB"
else
  mark "memory ceiling" "FAIL" "torrentngd process high-water RSS ${max_rss}MB > ${MAX_RSS_MB}MB"
fi

exit 0
