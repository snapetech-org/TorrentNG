#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/curl_policy.sh
source "$ROOT/scripts/curl_policy.sh"
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

CHURN_DOWNLOAD_DIR="transfer-churn-$FIXTURE_ID"
CATEGORY="cert-transfer-churn-$FIXTURE_ID"
TRACKER_NAME="tng-churn-tracker-$FIXTURE_ID"
FILESERVER_NAME="tng-churn-files-$FIXTURE_ID"
COOKIE_JAR=""
BODY=""
AUTH_BODY_FILE=""

mkdir -p "$(dirname "$OUT")"

mapped="$(docker port "$TNG_CONTAINER" 8080/tcp 2>/dev/null | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p' | head -1 || true)"
if [[ -n "$mapped" && "$TNG_HOST_URL" == http://localhost:* ]]; then
  TNG_HOST_URL="http://localhost:$mapped"
fi
python3 "$ROOT/scripts/protected_target.py" "$TNG_HOST_URL"
COOKIE_JAR="$(mktemp "${TMPDIR:-/tmp}/tng-churn-cookies.XXXXXX")"
BODY="$(mktemp "${TMPDIR:-/tmp}/tng-churn-body.XXXXXX")"
AUTH_BODY_FILE="$(mktemp "${TMPDIR:-/tmp}/tng-churn-auth.XXXXXX")"

status="PASS"
FAILURE_DIAGNOSTICS_CAPTURED="0"
seeders=()

mark() {
  local name="$1"
  local result="$2"
  local detail="$3"
  detail="${detail//$'\n'/ }"
  detail="${detail//|/\\|}"
  printf '| %s | %s | %s |\n' "$name" "$result" "$detail" >> "$OUT"
  [[ "$result" == "FAIL" ]] && status="FAIL"
  return 0
}

capture_failure_diagnostics() {
  [[ "$status" == "FAIL" && "$FAILURE_DIAGNOSTICS_CAPTURED" == "0" ]] || return 0
  FAILURE_DIAGNOSTICS_CAPTURED="1"
  {
    echo
    echo "## Failure Diagnostics"
    if [[ -s "$COOKIE_JAR" ]]; then
      local torrent_state
      torrent_state="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" \
        "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" 2>/dev/null \
        | jq -c '[.[] | {name,state,progress,downloaded,dlspeed,num_seeds,num_leechs,category}]' 2>/dev/null || true)"
      echo "- Torrent state: ${torrent_state:-unavailable}"
    fi
    echo
    echo "### TorrentNG daemon log tail"
    docker logs --tail 100 "$TNG_CONTAINER" 2>&1 || true
    local container
    for container in "$TRACKER_NAME" "$FILESERVER_NAME" "${seeders[@]}"; do
      [[ -n "$container" ]] || continue
      if docker inspect "$container" >/dev/null 2>&1; then
        echo
        echo "### $container"
        docker inspect --format 'state={{.State.Status}} exit={{.State.ExitCode}} error={{.State.Error}}' "$container" 2>/dev/null || true
        docker logs --tail 80 "$container" 2>&1 || true
      fi
    done
  } >> "$OUT"
}

cleanup() {
  capture_failure_diagnostics
  if [[ -s "$COOKIE_JAR" ]]; then
    delete_category_torrents || true
  fi
  if ((${#seeders[@]} > 0)); then
    for seeder in "${seeders[@]}"; do
      docker rm -f "$seeder" >/dev/null 2>&1 || true
    done
  fi
  docker rm -f "$FILESERVER_NAME" "$TRACKER_NAME" >/dev/null 2>&1 || true
  docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
    sh -c "rm -rf -- /downloads/$CHURN_DOWNLOAD_DIR" >/dev/null 2>&1 || true
  rm -f -- "$COOKIE_JAR" "$BODY" "$AUTH_BODY_FILE"
}
trap cleanup EXIT
tng_write_qbit_login_body "$TNG_API_TOKEN" "$AUTH_BODY_FILE"

http_code() {
  local url="$1"
  shift || true
  curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' "$@" "$url" || true
}

wait_for_container_dns() {
  local hostname="$1"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    if docker exec "$TNG_CONTAINER" getent hosts "$hostname" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

container_http_code() {
  local url="$1"
  docker exec "$TNG_CONTAINER" curl -q -sS --noproxy "*" --max-time 2 \
    -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || true
}

wait_for_container_http_status() {
  local url="$1"
  local expected="$2"
  local deadline=$((SECONDS + READY_TIMEOUT_SECS))
  local code
  while (( SECONDS < deadline )); do
    code="$(container_http_code "$url")"
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
    code="$(container_http_code "$url")"
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
    state="$(docker inspect --format '{{.State.Status}}' "$container" 2>/dev/null || true)"
    if [[ "$state" == "running" ]]; then
      logs="$(docker logs "$container" 2>&1 || true)"
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
  docker exec "$TNG_CONTAINER" cat "$torrent_path" | python3 -c '
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
  local encoded_hash
  encoded_hash="$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote_from_bytes(bytes.fromhex(sys.argv[1]), safe=""))' "$info_hash")"
  docker exec "$TNG_CONTAINER" curl -q -sS --noproxy "*" --max-time 2 \
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
    count="$(tracker_complete_count "$info_hash")"
    if [[ "$count" =~ ^[0-9]+$ ]] && (( count >= 1 )); then
      return 0
    fi
    sleep 1
  done
  return 1
}

runtime_owner() {
  local owner
  owner="$(docker exec "$TNG_CONTAINER" sh -lc '
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
  docker exec "$TNG_CONTAINER" sh -lc '
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
  local hashes
  hashes="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" \
    | jq -r '.[].hash' | paste -sd '|' -)"
  if [[ -n "$hashes" ]]; then
    curl -q -sS --noproxy "*" -b "$COOKIE_JAR" -X POST \
      -d "hashes=$hashes" \
      -d "deleteFiles=true" \
      "$TNG_HOST_URL/api/qb/v2/torrents/delete" >/dev/null || true
  fi
  return 0
}

read_disk_bytes() {
  local path="$1"
  docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 \
    sh -lc "if [ -f '$path' ]; then wc -c < '$path'; else printf 0; fi" 2>/dev/null \
    | tr -d '[:space:]' || echo 0
}

wait_for_completion() {
  local name="$1"
  local path="$2"
  local bytes="$3"
  local deadline=$((SECONDS + TRANSFER_TIMEOUT_SECS))
  while (( SECONDS < deadline )); do
    local row
    row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" \
      | jq -c --arg name "$name" '.[] | select(.name==$name)' | head -1 || true)"
    if [[ -n "$row" && "$(jq -r '.progress >= 1' <<<"$row")" == "true" ]]; then
      printf '%s' "$row"
      return 0
    fi
    local observed
    observed="$(read_disk_bytes "$path")"
    if [[ "$observed" == "$bytes" ]]; then
      printf '{"progress":1,"downloaded":%s}' "$observed"
      return 0
    fi
    sleep 2
  done
  return 1
}

{
  echo "# TorrentNG Transfer Churn Soak"
  echo
  echo "- Date UTC: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- TorrentNG URL: $TNG_HOST_URL"
  echo "- Docker network: $COMPOSE_NETWORK"
  echo "- Downloads volume: $DOWNLOADS_VOLUME"
  echo "- Local cycles: $CHURN_CYCLES"
  echo "- Fixture bytes per cycle: $FIXTURE_BYTES"
  echo "- Public Linux torrent cycles: $PUBLIC_CYCLES"
  [[ "$PUBLIC_CYCLES" != "0" ]] && echo "- Public torrent URL: $PUBLIC_TORRENT_URL"
  echo "- Transfer timeout seconds: $TRANSFER_TIMEOUT_SECS"
  echo "- Max RSS MB: $MAX_RSS_MB"
  echo
  echo "## Checks"
  echo
  echo "| Check | Result | Detail |"
  echo "|---|---|---|"
} > "$OUT"

code="$(http_code "$TNG_HOST_URL/api/qb/v2/auth/login" -X POST --data-binary "@$AUTH_BODY_FILE" -c "$COOKIE_JAR")"
if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]]; then
  mark "qBit auth" "PASS" "session cookie accepted"
else
  mark "qBit auth" "FAIL" "HTTP $code body=$(tr '\n' ' ' <"$BODY")"
  echo >> "$OUT"; echo "Overall status: $status" >> "$OUT"; echo "$OUT"; exit 1
fi

delete_category_torrents

docker run -d --name "$TRACKER_NAME" --network "$COMPOSE_NETWORK" alpine:3.20 \
  sh -lc 'apk add --no-cache opentracker >/dev/null && exec opentracker -i 0.0.0.0 -p 6969 -P 6969' >/dev/null
if wait_for_container_dns "$TRACKER_NAME" \
  && wait_for_container_http_response "http://$TRACKER_NAME:6969/announce"; then
  mark "local tracker" "PASS" "DNS and HTTP ready at http://$TRACKER_NAME:6969/announce"
else
  mark "local tracker" "FAIL" "DNS/HTTP readiness timed out after ${READY_TIMEOUT_SECS}s"
  echo >> "$OUT"; echo "Overall status: $status" >> "$OUT"; echo "$OUT"; exit 1
fi

APP_RUNTIME_OWNER="$(runtime_owner)" || {
  mark "app runtime identity" "FAIL" "could not read torrentngd UID/GID from the container"
  echo >> "$OUT"; echo "Overall status: $status" >> "$OUT"; echo "$OUT"; exit 1
}
mark "app runtime identity" "PASS" "torrentngd runs as $APP_RUNTIME_OWNER"

docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
  sh -c "mkdir -p -- /downloads/$CHURN_DOWNLOAD_DIR"

docker run -d --name "$FILESERVER_NAME" --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.20 \
  sh -lc "apk add --no-cache busybox-extras >/dev/null && exec httpd -f -p 8081 -h /downloads/$CHURN_DOWNLOAD_DIR" >/dev/null
if wait_for_container_dns "$FILESERVER_NAME" \
  && wait_for_container_http_response "http://$FILESERVER_NAME:8081/"; then
  mark "fixture file server" "PASS" "DNS and HTTP ready at http://$FILESERVER_NAME:8081/"
else
  mark "fixture file server" "FAIL" "DNS/HTTP readiness timed out after ${READY_TIMEOUT_SECS}s"
  echo >> "$OUT"; echo "Overall status: $status" >> "$OUT"; echo "$OUT"; exit 1
fi

{
  echo
  echo "## Transfer Cycles"
  echo
  echo "| Cycle | Type | Seeder tracked | Add HTTP | Progress | Downloaded | RSS MB | Result |"
  echo "|---:|---|---|---:|---:|---:|---:|---|"
} >> "$OUT"

max_rss="0"
for cycle in $(seq 1 "$CHURN_CYCLES"); do
  name="tng-churn-$FIXTURE_ID-$cycle.bin"
  torrent="tng-churn-$FIXTURE_ID-$cycle.torrent"
  seeder="tng-churn-seeder-$FIXTURE_ID-$cycle"
  seeders+=("$seeder")

  docker run --rm --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 sh -lc "
    set -e
    apk add --no-cache mktorrent >/dev/null
    mkdir -p /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed /downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech
    chown $APP_RUNTIME_OWNER /downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech
    dd if=/dev/urandom of=/downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed/$name bs=$FIXTURE_BYTES count=1 status=none
    mktorrent -a http://$TRACKER_NAME:6969/announce -o /downloads/$CHURN_DOWNLOAD_DIR/$cycle/$torrent /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed/$name >/dev/null
  "

  docker run -d --name "$seeder" --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
    sh -lc "apk add --no-cache transmission-cli >/dev/null && exec transmission-cli -M -et -w /downloads/$CHURN_DOWNLOAD_DIR/$cycle/seed /downloads/$CHURN_DOWNLOAD_DIR/$cycle/$torrent" >/dev/null

  fixture_url="http://$FILESERVER_NAME:8081/$cycle/$torrent"
  torrent_path="/data/$CHURN_DOWNLOAD_DIR/$cycle/$torrent"
  if wait_for_seeder_listener "$seeder" \
    && wait_for_container_http_status "$fixture_url" 200 \
    && info_hash="$(torrent_info_hash "$torrent_path")" \
    && wait_for_tracker_seed "$info_hash"; then
    seeder_tracked="PASS"
    code="$(curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
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
  result="FAIL"
  if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]] && row="$(wait_for_completion "$name" "/downloads/$CHURN_DOWNLOAD_DIR/$cycle/leech/$name" "$FIXTURE_BYTES")"; then
    progress="$(jq -r '.progress' <<<"$row")"
    downloaded="$(jq -r '.downloaded' <<<"$row")"
    result="PASS"
  fi

  rss="$(rss_mb)"
  awk -v a="$rss" -v b="$max_rss" 'BEGIN {exit !(a > b)}' && max_rss="$rss"
  printf '| %s | local fixture | %s | %s | %s | %s | %s | %s |\n' \
    "$cycle" "$seeder_tracked" "$code" "$progress" "$downloaded" "$rss" "$result" >> "$OUT"
  if [[ "$result" != "PASS" ]]; then
    status="FAIL"
    capture_failure_diagnostics
  fi

  delete_category_torrents
  docker rm -f "$seeder" >/dev/null 2>&1 || true
  docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
    sh -c "rm -rf -- /downloads/$CHURN_DOWNLOAD_DIR/$cycle" >/dev/null 2>&1 || true
done

for cycle in $(seq 1 "$PUBLIC_CYCLES"); do
  code="$(curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
    -F "urls=$PUBLIC_TORRENT_URL" \
    -F "savepath=/data/$CHURN_DOWNLOAD_DIR/public/$cycle" \
    -F "category=$CATEGORY" \
    -F "stopped=false" \
    "$TNG_HOST_URL/api/qb/v2/torrents/add" || true)"
  sleep 10
  row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$CATEGORY" | jq -c '.[0] // empty' || true)"
  progress="$(jq -r '.progress // 0' <<<"${row:-{}}")"
  downloaded="$(jq -r '.downloaded // 0' <<<"${row:-{}}")"
  rss="$(rss_mb)"
  awk -v a="$rss" -v b="$max_rss" 'BEGIN {exit !(a > b)}' && max_rss="$rss"
  result="$([[ "$code" == "200" && -n "$row" ]] && echo PASS || echo FAIL)"
  printf '| %s | public Linux torrent | %s | %s | %s | %s | %s |\n' "$cycle" "$code" "$progress" "$downloaded" "$rss" "$result" >> "$OUT"
  if [[ "$result" != "PASS" ]]; then
    status="FAIL"
    capture_failure_diagnostics
  fi
  delete_category_torrents
  docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.20 \
    sh -c "rm -rf -- /downloads/$CHURN_DOWNLOAD_DIR/public/$cycle" >/dev/null 2>&1 || true
done

if awk -v rss="$max_rss" -v limit="$MAX_RSS_MB" 'BEGIN {exit !(rss <= limit)}'; then
  mark "memory ceiling" "PASS" "torrentngd process high-water RSS ${max_rss}MB <= ${MAX_RSS_MB}MB"
else
  mark "memory ceiling" "FAIL" "torrentngd process high-water RSS ${max_rss}MB > ${MAX_RSS_MB}MB"
fi

{
  echo
  echo "Overall status: $status"
} >> "$OUT"

echo "$OUT"
[[ "$status" == "PASS" ]]
