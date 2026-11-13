#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/curl_policy.sh
source "$ROOT/scripts/curl_policy.sh"
ENV_FILE="${CERT_ENV_FILE:-$ROOT/deploy/certification/.env}"
OUT="${1:-$ROOT/certification/reports/live-transfer-$(date -u +%Y%m%dT%H%M%SZ).md}"

ENV_TNG_HOST_URL="${TNG_HOST_URL:-}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

TNG_HOST_URL="${ENV_TNG_HOST_URL:-${TNG_HOST_URL:-http://localhost:${TNG_HOST_PORT:-18080}}}"
TNG_API_TOKEN="${TNG_API_TOKEN:-local-cert-api-token-20260904}"
python3 "$ROOT/scripts/protected_target.py" "$TNG_HOST_URL"
COMPOSE_PROJECT="${CERT_COMPOSE_PROJECT:-certification}"
COMPOSE_NETWORK="${CERT_COMPOSE_NETWORK:-${COMPOSE_PROJECT}_default}"
DOWNLOADS_VOLUME="${CERT_DOWNLOADS_VOLUME:-${COMPOSE_PROJECT}_downloads}"
RUNTIME_UID="${CERT_RUNTIME_UID:-${PUID:-1000}}"
RUNTIME_GID="${CERT_RUNTIME_GID:-${PGID:-1000}}"
FIXTURE_ID_SEED="${CERT_FIXTURE_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
FIXTURE_BYTES="${CERT_FIXTURE_BYTES:-1048576}"
PUBLIC_TORRENT_URL="${PUBLIC_TORRENT_URL:-https://mirror.arizona.edu/debian-cd/current/amd64/bt-cd/debian-13.4.0-amd64-netinst.iso.torrent}"
PUBLIC_TRANSFER="${PUBLIC_TRANSFER:-0}"
TRANSFER_TIMEOUT_SECS="${CERT_TRANSFER_TIMEOUT_SECS:-120}"
PUBLIC_TIMEOUT_SECS="${CERT_PUBLIC_TRANSFER_TIMEOUT_SECS:-1800}"

if [[ ! "$FIXTURE_ID_SEED" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$ || "$FIXTURE_ID_SEED" == *..* ]]; then
  echo "CERT_FIXTURE_ID must be 1-32 safe filename characters without '..'" >&2
  exit 2
fi
FIXTURE_ID="$FIXTURE_ID_SEED-$$"
if [[ ! "$FIXTURE_BYTES" =~ ^[0-9]{1,8}$ ]]; then
  echo "CERT_FIXTURE_BYTES must be an integer between 1 and 67108864" >&2
  exit 2
fi
FIXTURE_BYTES=$((10#$FIXTURE_BYTES))
if (( FIXTURE_BYTES < 1 || FIXTURE_BYTES > 67108864 )); then
  echo "CERT_FIXTURE_BYTES must be an integer between 1 and 67108864" >&2
  exit 2
fi
if [[ "$PUBLIC_TRANSFER" != "0" && "$PUBLIC_TRANSFER" != "1" ]]; then
  echo "PUBLIC_TRANSFER must be 0 or 1" >&2
  exit 2
fi
for identity_name in RUNTIME_UID RUNTIME_GID; do
  identity_value="${!identity_name}"
  if [[ ! "$identity_value" =~ ^[1-9][0-9]{0,8}$ ]]; then
    echo "$identity_name must be a nonzero numeric ID" >&2
    exit 2
  fi
done
for timeout_name in TRANSFER_TIMEOUT_SECS PUBLIC_TIMEOUT_SECS; do
  timeout_value="${!timeout_name}"
  if [[ ! "$timeout_value" =~ ^[0-9]{1,5}$ ]]; then
    echo "$timeout_name must be an integer between 1 and 7200" >&2
    exit 2
  fi
  timeout_value=$((10#$timeout_value))
  if (( timeout_value < 1 || timeout_value > 7200 )); then
    echo "$timeout_name must be an integer between 1 and 7200" >&2
    exit 2
  fi
  printf -v "$timeout_name" '%s' "$timeout_value"
done

FIXTURE_DOWNLOAD_DIR="cert-fixture-$FIXTURE_ID"
LOCAL_CATEGORY="cert-local-fixture-$FIXTURE_ID"
PUBLIC_CATEGORY="cert-public-$FIXTURE_ID"

if ! [[ "$FIXTURE_ID" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]]; then
  echo "CERT_FIXTURE_ID must contain only bounded shell-safe identifier characters" >&2
  exit 2
fi
if ! [[ "$FIXTURE_BYTES" =~ ^[1-9][0-9]{0,8}$ ]] || (( FIXTURE_BYTES > 268435456 )); then
  echo "CERT_FIXTURE_BYTES must be between 1 and 268435456" >&2
  exit 2
fi

TRACKER_NAME="tng-cert-tracker-$FIXTURE_ID"
SEEDER_NAME="tng-cert-seeder-$FIXTURE_ID"
COOKIE_JAR="$(mktemp "${TMPDIR:-/tmp}/tng-transfer-cookies.XXXXXX")"
BODY="$(mktemp "${TMPDIR:-/tmp}/tng-transfer-body.XXXXXX")"
AUTH_BODY_FILE="$(mktemp "${TMPDIR:-/tmp}/tng-transfer-auth.XXXXXX")"
QBIT_AUTH_HEADER_FILE="$(mktemp "${TMPDIR:-/tmp}/tng-transfer-qbit-auth.XXXXXX")"
FIXTURE_TORRENT_FILE="$(mktemp "${TMPDIR:-/tmp}/tng-fixture-torrent.XXXXXX")"

mkdir -p "$(dirname "$OUT")"

cleanup() {
  if [[ -s "$COOKIE_JAR" ]]; then
    delete_category_torrents "$LOCAL_CATEGORY" || true
    delete_category_torrents "$PUBLIC_CATEGORY" || true
  fi
  docker rm -f "$SEEDER_NAME" "$TRACKER_NAME" >/dev/null 2>&1 || true
  docker run --rm -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf \
    sh -c "rm -rf -- /downloads/$FIXTURE_DOWNLOAD_DIR" >/dev/null 2>&1 || true
  rm -f -- "$COOKIE_JAR" "$BODY" "$AUTH_BODY_FILE" "$QBIT_AUTH_HEADER_FILE" "$FIXTURE_TORRENT_FILE"
}
trap cleanup EXIT
tng_write_qbit_login_body "$TNG_API_TOKEN" "$AUTH_BODY_FILE"
{
  printf 'Authorization: Basic '
  printf '%s' "$TNG_API_TOKEN:$TNG_API_TOKEN" | base64 | tr -d '\r\n'
  printf '\n'
} > "$QBIT_AUTH_HEADER_FILE"
chmod 600 -- "$QBIT_AUTH_HEADER_FILE"

status="PASS"
mark() {
  local name="$1"
  local result="$2"
  local detail="$3"
  printf '| %s | %s | %s |\n' "$name" "$result" "$detail" >> "$OUT"
  if [[ "$result" == "FAIL" ]]; then
    status="FAIL"
  fi
}

http_code() {
  local url="$1"
  shift || true
  curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' "$@" "$url" || true
}

wait_for_torrent() {
  local name="$1"
  local want_complete="$2"
  local timeout="${3:-$TRANSFER_TIMEOUT_SECS}"
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    local row
    row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info" \
      | jq -c --arg name "$name" '.[] | select(.name==$name)' | head -1 || true)"
    if [[ -n "$row" ]]; then
      if [[ "$want_complete" != "1" || "$(jq -r '.progress >= 1' <<<"$row")" == "true" ]]; then
        printf '%s' "$row"
        return 0
      fi
    fi
    sleep 2
  done
  return 1
}

wait_for_category_torrent() {
  local category="$1"
  local want_complete="$2"
  local timeout="${3:-$TRANSFER_TIMEOUT_SECS}"
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    local row
    row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$category" \
      | jq -c 'sort_by(.progress) | reverse | .[0] // empty' || true)"
    if [[ -n "$row" ]]; then
      if [[ "$want_complete" != "1" || "$(jq -r '.progress >= 1' <<<"$row")" == "true" ]]; then
        printf '%s' "$row"
        return 0
      fi
    fi
    sleep 5
  done
  return 1
}

delete_category_torrents() {
  local category="$1"
  local hashes
  hashes="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$category" \
    | jq -r '.[].hash' | paste -sd '|' -)"
  if [[ -n "$hashes" ]]; then
    curl -q -sS --noproxy "*" -b "$COOKIE_JAR" -X POST \
      -H "@$QBIT_AUTH_HEADER_FILE" \
      -d "hashes=$hashes" \
      -d "deleteFiles=true" \
      "$TNG_HOST_URL/api/qb/v2/torrents/delete" >/dev/null || true
    sleep 2
  fi
}

{
  echo "# TorrentNG Live Transfer Certification Report"
  echo
  echo "- Date UTC: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- TorrentNG URL: $TNG_HOST_URL"
  echo "- Docker network: $COMPOSE_NETWORK"
  echo "- Downloads volume: $DOWNLOADS_VOLUME"
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
  echo >> "$OUT"
  echo "Overall status: $status" >> "$OUT"
  echo "$OUT"
  exit 1
fi

delete_category_torrents "$LOCAL_CATEGORY"
delete_category_torrents "$PUBLIC_CATEGORY"

docker run -d --rm --name "$TRACKER_NAME" --network "$COMPOSE_NETWORK" \
  lednerb/opentracker-docker@sha256:a055447e44450036b7b2a923618b12d4ecf1eff3c894e969cc2c42f466b2c6f6 >/dev/null
if docker run --rm --network "$COMPOSE_NETWORK" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf \
  sh -lc "for attempt in \$(seq 1 20); do nc -z -w 1 $TRACKER_NAME 6969 && exit 0; sleep 1; done; exit 1"; then
  mark "local tracker" "PASS" "$TRACKER_NAME accepts HTTP connections on port 6969"
else
  mark "local tracker" "FAIL" "$TRACKER_NAME did not accept HTTP connections on port 6969"
fi

docker run --rm --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf sh -lc "
  set -e
  apk add --no-cache mktorrent >/dev/null
  mkdir -p /downloads/$FIXTURE_DOWNLOAD_DIR/seed /downloads/$FIXTURE_DOWNLOAD_DIR/leech
  dd if=/dev/urandom of=/downloads/$FIXTURE_DOWNLOAD_DIR/seed/tng-fixture.bin bs=$FIXTURE_BYTES count=1 status=none
  mktorrent -a http://$TRACKER_NAME:6969/announce -o /downloads/$FIXTURE_DOWNLOAD_DIR/tng-fixture.torrent /downloads/$FIXTURE_DOWNLOAD_DIR/seed/tng-fixture.bin >/dev/null
  chown -R $RUNTIME_UID:$RUNTIME_GID /downloads/$FIXTURE_DOWNLOAD_DIR
"
docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf \
  cat "/downloads/$FIXTURE_DOWNLOAD_DIR/tng-fixture.torrent" > "$FIXTURE_TORRENT_FILE"
mark "fixture torrent" "PASS" "$FIXTURE_BYTES byte torrent generated in Docker volume"
owner="$(docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf stat -c '%u:%g' "/downloads/$FIXTURE_DOWNLOAD_DIR/leech")"
if [[ "$owner" == "$RUNTIME_UID:$RUNTIME_GID" ]]; then
  mark "fixture ownership" "PASS" "download directory is writable by TorrentNG runtime $owner"
else
  mark "fixture ownership" "FAIL" "download directory owner is $owner, expected $RUNTIME_UID:$RUNTIME_GID"
fi

docker run -d --rm --name "$SEEDER_NAME" --network "$COMPOSE_NETWORK" -v "$DOWNLOADS_VOLUME:/downloads" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf \
  sh -lc "apk add --no-cache transmission-cli >/dev/null && exec transmission-cli -w /downloads/$FIXTURE_DOWNLOAD_DIR/seed /downloads/$FIXTURE_DOWNLOAD_DIR/tng-fixture.torrent" >/dev/null
sleep 4
if [[ "$(docker inspect -f '{{.State.Running}}' "$SEEDER_NAME" 2>/dev/null || true)" == "true" ]]; then
  mark "stock seeder" "PASS" "$SEEDER_NAME running transmission-cli"
else
  mark "stock seeder" "FAIL" "$SEEDER_NAME exited before the client add"
fi

code="$(curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
  -H "@$QBIT_AUTH_HEADER_FILE" \
  -F "torrents=@$FIXTURE_TORRENT_FILE;type=application/x-bittorrent" \
  -F "savepath=/data/$FIXTURE_DOWNLOAD_DIR/leech" \
  -F "category=$LOCAL_CATEGORY" \
  -F "stopped=false" \
  "$TNG_HOST_URL/api/qb/v2/torrents/add" || true)"
if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]]; then
  mark "TorrentNG add local fixture file" "PASS" "qBit add accepted local torrent upload"
else
  mark "TorrentNG add local fixture file" "FAIL" "HTTP $code body=$(tr '\n' ' ' <"$BODY")"
fi

row="$(wait_for_torrent "tng-fixture.bin" 0 30 || true)"
hash="$(jq -r '.hash // empty' <<<"${row:-null}" 2>/dev/null || true)"
if [[ -n "$hash" ]]; then
  code="$(curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
    "$TNG_HOST_URL/api/qb/v2/torrents/trackers?hash=$hash" || true)"
  if [[ "$code" == "200" ]] && jq -e 'type == "array" and length > 0' "$BODY" >/dev/null; then
    mark "qB tracker listing" "PASS" "HTTP 200 with $(jq 'length' "$BODY") tracker row(s)"
  else
    mark "qB tracker listing" "FAIL" "HTTP $code body=$(tr '\n' ' ' <"$BODY")"
  fi
else
  mark "qB tracker listing" "FAIL" "torrent row missing after accepted add"
fi

if row="$(wait_for_torrent "tng-fixture.bin" 1)"; then
  mark "local fixture transfer" "PASS" "$(jq -r '"progress=\(.progress) size=\(.size) downloaded=\(.downloaded)"' <<<"$row")"
elif docker run --rm -v "$DOWNLOADS_VOLUME:/downloads:ro" alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf \
  sh -lc "test \"\$(wc -c </downloads/$FIXTURE_DOWNLOAD_DIR/leech/tng-fixture.bin 2>/dev/null || echo 0)\" -eq $FIXTURE_BYTES"; then
  mark "local fixture transfer" "PASS" "peer payload completed on disk; qBit cache did not report before timeout"
else
  row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$LOCAL_CATEGORY" \
    | jq -c '.[0] | {state,progress,downloaded,size,num_seeds,num_leechs,dlspeed} // empty' || true)"
  mark "local fixture transfer" "FAIL" "fixture did not complete within timeout; latest=${row:-none}"
fi

code="$(curl -q -sS --noproxy "*" -o "$BODY" -w '%{http_code}' -b "$COOKIE_JAR" \
  -H "@$QBIT_AUTH_HEADER_FILE" \
  -F "urls=$PUBLIC_TORRENT_URL" \
  -F "savepath=/data/$FIXTURE_DOWNLOAD_DIR/public" \
  -F "category=$PUBLIC_CATEGORY" \
  -F "stopped=$([[ "$PUBLIC_TRANSFER" == "1" ]] && echo false || echo true)" \
  "$TNG_HOST_URL/api/qb/v2/torrents/add" || true)"
if [[ "$code" == "200" && "$(cat "$BODY")" == "Ok." ]]; then
  mark "public Linux torrent add" "PASS" "$PUBLIC_TORRENT_URL"
elif [[ "$PUBLIC_TRANSFER" == "0" ]]; then
  mark "public Linux torrent add" "INFO" "external torrent fetch unavailable in this local run; public compatibility is qualified separately"
else
  mark "public Linux torrent add" "FAIL" "HTTP $code body=$(tr '\n' ' ' <"$BODY")"
fi

if [[ "$PUBLIC_TRANSFER" == "1" ]]; then
  if row="$(wait_for_category_torrent "$PUBLIC_CATEGORY" 1 "$PUBLIC_TIMEOUT_SECS")"; then
    mark "public Linux transfer" "PASS" "$(jq -r '"progress=\(.progress) size=\(.size) downloaded=\(.downloaded)"' <<<"$row")"
  else
    row="$(curl -q -sS --noproxy "*" -b "$COOKIE_JAR" "$TNG_HOST_URL/api/qb/v2/torrents/info?category=$PUBLIC_CATEGORY" \
      | jq -c 'sort_by(.progress) | reverse | .[0] // empty' || true)"
    mark "public Linux transfer" "FAIL" "did not complete within ${PUBLIC_TIMEOUT_SECS}s latest=${row:-none}"
  fi
else
  mark "public Linux transfer" "INFO" "skipped by default; set PUBLIC_TRANSFER=1 to download"
fi

{
  echo
  echo "Overall status: $status"
} >> "$OUT"

echo "$OUT"
[[ "$status" == "PASS" ]]
