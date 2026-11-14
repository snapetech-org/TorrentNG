#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/curl_policy.sh
source "$ROOT/scripts/curl_policy.sh"
ENV_FILE="${CERT_ENV_FILE:-$ROOT/deploy/certification/.env}"
OUT="${1:-$ROOT/certification/reports/client-config-$(date -u +%Y%m%dT%H%M%SZ).md}"

if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

TNG_API_TOKEN="${TNG_API_TOKEN:-local-cert-api-token-20260904}"
CERT_TNG_CLIENT_HOST="${CERT_TNG_CLIENT_HOST:-torrentng}"
CERT_TNG_CLIENT_PORT="${CERT_TNG_CLIENT_PORT:-8080}"
CERT_TNG_CLIENT_TLS="${CERT_TNG_CLIENT_TLS:-false}"
if [[ ! "$CERT_TNG_CLIENT_PORT" =~ ^[0-9]{1,5}$ ]] || (( CERT_TNG_CLIENT_PORT < 1 || CERT_TNG_CLIENT_PORT > 65535 )); then
  echo "CERT_TNG_CLIENT_PORT must be between 1 and 65535" >&2
  exit 2
fi
if [[ "$CERT_TNG_CLIENT_TLS" != true && "$CERT_TNG_CLIENT_TLS" != false ]]; then
  echo "CERT_TNG_CLIENT_TLS must be true or false" >&2
  exit 2
fi
SONARR_HOST_URL="${SONARR_HOST_URL:-http://localhost:${SONARR_HOST_PORT:-18989}}"
RADARR_HOST_URL="${RADARR_HOST_URL:-http://localhost:${RADARR_HOST_PORT:-17878}}"
PROWLARR_HOST_URL="${PROWLARR_HOST_URL:-http://localhost:${PROWLARR_HOST_PORT:-19696}}"
AUTOBRR_HOST_URL="${AUTOBRR_HOST_URL:-http://localhost:${AUTOBRR_HOST_PORT:-17474}}"

SONARR_CONTAINER="${SONARR_CONTAINER:-certification-sonarr-1}"
RADARR_CONTAINER="${RADARR_CONTAINER:-certification-radarr-1}"
PROWLARR_CONTAINER="${PROWLARR_CONTAINER:-certification-prowlarr-1}"
AUTOBRR_CONTAINER="${AUTOBRR_CONTAINER:-certification-autobrr-1}"

mapped_host_url() {
  local current="$1"
  local container="$2"
  local container_port="$3"
  local mapped

  mapped="$(docker port "$container" "$container_port/tcp" 2>/dev/null | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p' | head -1 || true)"
  if [[ -n "$mapped" && "$current" == http://localhost:* ]]; then
    printf 'http://localhost:%s\n' "$mapped"
  else
    printf '%s\n' "$current"
  fi
}

SONARR_HOST_URL="$(mapped_host_url "$SONARR_HOST_URL" "$SONARR_CONTAINER" 8989)"
RADARR_HOST_URL="$(mapped_host_url "$RADARR_HOST_URL" "$RADARR_CONTAINER" 7878)"
PROWLARR_HOST_URL="$(mapped_host_url "$PROWLARR_HOST_URL" "$PROWLARR_CONTAINER" 9696)"
AUTOBRR_HOST_URL="$(mapped_host_url "$AUTOBRR_HOST_URL" "$AUTOBRR_CONTAINER" 7474)"

for protected_url in "$SONARR_HOST_URL" "$RADARR_HOST_URL" \
  "$PROWLARR_HOST_URL" "$AUTOBRR_HOST_URL"; do
  python3 "$ROOT/scripts/protected_target.py" "$protected_url"
done

mkdir -p "$(dirname "$OUT")"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/torrentng-client-config.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

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

api_key_from_container() {
  local container="$1"
  docker exec "$container" sh -lc "sed -n 's:.*<ApiKey>\\(.*\\)</ApiKey>.*:\\1:p' /config/config.xml | head -1"
}

arr_payload() {
  local base_url="$1"
  local api_key="$2"
  local api_path="$3"
  local name="$4"
  local category_field="$5"
  local category_value="$6"

  local tls_json=false
  [[ "$CERT_TNG_CLIENT_TLS" == true ]] && tls_json=true
  curl -fsS -H "X-Api-Key: $api_key" "$base_url$api_path/downloadclient/schema" \
    | jq --arg name "$name" --arg category_field "$category_field" --arg category_value "$category_value" \
        --arg token "$TNG_API_TOKEN" --arg host "$CERT_TNG_CLIENT_HOST" \
        --argjson port "$CERT_TNG_CLIENT_PORT" --argjson tls "$tls_json" '
      map(select(.implementation=="QBittorrent"))[0]
      | .enable=true
      | .name=$name
      | .priority=1
      | .fields |= map(
          if .name=="host" then .value=$host
          elif .name=="port" then .value=$port
          elif .name=="useSsl" then .value=$tls
          elif .name=="urlBase" then .value=""
          elif .name=="apiKey" then .value=$token
          elif .name=="username" then .value=$token
          elif .name=="password" then .value=$token
          elif .name==$category_field then .value=$category_value
          elif .name=="initialState" then .value=0
          elif .name=="sequentialOrder" then .value=false
          elif .name=="firstAndLast" then .value=false
          elif .name=="contentLayout" then .value=0
          else . end
        )
      | if any(.fields[]; .name=="apiKey") then
          .fields |= map(
            if .name=="username" or .name=="password" then .value=""
            else . end
          )
        else . end'
}

configure_arr_client() {
  local label="$1"
  local base_url="$2"
  local api_key="$3"
  local api_path="$4"
  local category_field="$5"
  local category_value="$6"
  local name="TorrentNG-qBit"
  local test_body="$TMP_DIR/arr-test-body"
  local save_body="$TMP_DIR/arr-save-body"
  local payload code existing_id detail

  payload="$(arr_payload "$base_url" "$api_key" "$api_path" "$name" "$category_field" "$category_value")"
  existing_id="$(curl -fsS -H "X-Api-Key: $api_key" "$base_url$api_path/downloadclient" | jq -r --arg name "$name" '.[] | select(.name==$name) | .id' | head -1)"
  if [[ -n "$existing_id" ]]; then
    payload="$(printf '%s' "$payload" | jq --argjson id "$existing_id" '.id=$id')"
  fi

  code="$(curl -q -sS --noproxy "*" -o "$test_body" -w '%{http_code}' -H "X-Api-Key: $api_key" -H 'Content-Type: application/json' -X POST -d "$payload" "$base_url$api_path/downloadclient/test")"
  if [[ ! "$code" =~ ^(200|202|204)$ ]]; then
    detail="$(tr '\n' ' ' <"$test_body")"
    detail="${detail//"$TNG_API_TOKEN"/<redacted>}"
    mark "$label qBit test" "FAIL" "HTTP $code $detail"
    return
  fi

  if [[ -n "$existing_id" ]]; then
    code="$(curl -q -sS --noproxy "*" -o "$save_body" -w '%{http_code}' -H "X-Api-Key: $api_key" -H 'Content-Type: application/json' -X PUT -d "$payload" "$base_url$api_path/downloadclient/$existing_id")"
  else
    code="$(curl -q -sS --noproxy "*" -o "$save_body" -w '%{http_code}' -H "X-Api-Key: $api_key" -H 'Content-Type: application/json' -X POST -d "$payload" "$base_url$api_path/downloadclient")"
  fi

  if [[ "$code" == "200" || "$code" == "201" || "$code" == "202" ]]; then
    mark "$label qBit client" "PASS" "tested and saved as $name"
  else
    detail="$(tr '\n' ' ' <"$save_body")"
    detail="${detail//"$TNG_API_TOKEN"/<redacted>}"
    mark "$label qBit client" "FAIL" "HTTP $code $detail"
  fi
}

configure_autobrr_client() {
  local cookies="$TMP_DIR/autobrr-cookies"
  local onboard_body="$TMP_DIR/autobrr-onboard-body"
  local login_body="$TMP_DIR/autobrr-login-body"
  local test_body="$TMP_DIR/autobrr-test-body"
  local save_body="$TMP_DIR/autobrr-save-body"
  local user="${AUTOBRR_CERT_USER:-cert}"
  local pass="${AUTOBRR_CERT_PASSWORD:-cert}"
  local payload code existing_id detail
  : >"$cookies"

  code="$(curl -q -sS --noproxy "*" -o "$onboard_body" -w '%{http_code}' "$AUTOBRR_HOST_URL/api/auth/onboard")"
  if [[ "$code" == "204" ]]; then
    curl -q -sS --noproxy "*" -o "$onboard_body" -H 'Content-Type: application/json' -X POST -d "{\"username\":\"$user\",\"password\":\"$pass\"}" "$AUTOBRR_HOST_URL/api/auth/onboard" >/dev/null
  fi

  code="$(curl -q -sS --noproxy "*" -c "$cookies" -o "$login_body" -w '%{http_code}' -H 'Content-Type: application/json' -X POST -d "{\"username\":\"$user\",\"password\":\"$pass\",\"remember_me\":true}" "$AUTOBRR_HOST_URL/api/auth/login")"
  if [[ "$code" != "204" ]]; then
    detail="$(tr '\n' ' ' <"$login_body")"
    mark "autobrr login" "FAIL" "HTTP $code $detail"
    return
  fi

  local tls_json=false
  [[ "$CERT_TNG_CLIENT_TLS" == true ]] && tls_json=true
  # Use the API-token bearer path when supported so HTTPS-only SID cookies remain secure.
  payload="$(jq -nc --arg token "$TNG_API_TOKEN" --arg host "$CERT_TNG_CLIENT_HOST" \
    --argjson port "$CERT_TNG_CLIENT_PORT" --argjson tls "$tls_json" '{
    name: "TorrentNG-qBit",
    type: "QBITTORRENT",
    enabled: true,
    host: $host,
    port: $port,
    tls: $tls,
    tls_skip_verify: false,
    username: "",
    password: "",
    settings: {
      apikey: $token,
      basic: {auth: false},
      rules: {enabled: false}
    }
  }')"

  code="$(curl -q -sS --noproxy "*" -b "$cookies" -o "$test_body" -w '%{http_code}' -H 'Content-Type: application/json' -X POST -d "$payload" "$AUTOBRR_HOST_URL/api/downloaders/test")"
  if [[ "$code" != "204" ]]; then
    detail="$(tr '\n' ' ' <"$test_body")"
    detail="${detail//"$TNG_API_TOKEN"/<redacted>}"
    mark "autobrr qBit test" "FAIL" "HTTP $code $detail"
    return
  fi

  existing_id="$(curl -fsS -b "$cookies" "$AUTOBRR_HOST_URL/api/downloaders" | jq -r '.[] | select(.name=="TorrentNG-qBit") | .id' | head -1)"
  if [[ -n "$existing_id" ]]; then
    payload="$(printf '%s' "$payload" | jq --argjson id "$existing_id" '.id=$id')"
    code="$(curl -q -sS --noproxy "*" -b "$cookies" -o "$save_body" -w '%{http_code}' -H 'Content-Type: application/json' -X PUT -d "$payload" "$AUTOBRR_HOST_URL/api/downloaders")"
  else
    code="$(curl -q -sS --noproxy "*" -b "$cookies" -o "$save_body" -w '%{http_code}' -H 'Content-Type: application/json' -X POST -d "$payload" "$AUTOBRR_HOST_URL/api/downloaders")"
  fi

  if [[ "$code" == "200" || "$code" == "201" ]]; then
    mark "autobrr qBit client" "PASS" "tested and saved as TorrentNG-qBit"
  else
    detail="$(tr '\n' ' ' <"$save_body")"
    detail="${detail//"$TNG_API_TOKEN"/<redacted>}"
    mark "autobrr qBit client" "FAIL" "HTTP $code $detail"
  fi
}

{
  echo "# TorrentNG Client Configuration Report"
  echo
  echo "- Date UTC: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  if [[ "$CERT_TNG_CLIENT_TLS" == true ]]; then compat_scheme=https; else compat_scheme=http; fi
  echo "- TorrentNG qBittorrent endpoint: ${compat_scheme}://${CERT_TNG_CLIENT_HOST}:${CERT_TNG_CLIENT_PORT}"
  echo
  echo "## Checks"
  echo
  echo "| Check | Result | Detail |"
  echo "|---|---|---|"
} > "$OUT"

if [[ "$CERT_TNG_CLIENT_TLS" == true ]]; then
  if [[ -z "${CERT_TLS_DIR:-}" || -z "${COMPOSE_PROJECT_NAME:-}" || ! -s "$CERT_TLS_DIR/ca.crt" ]]; then
    mark "certification TLS trust" "FAIL" "TLS mode requires CERT_TLS_DIR and COMPOSE_PROJECT_NAME for the isolated stack"
    status="FAIL"
  elif "$ROOT/scripts/install_certification_test_ca.sh" "$COMPOSE_PROJECT_NAME" "$CERT_TLS_DIR/ca.crt" \
      "$SONARR_CONTAINER" "$RADARR_CONTAINER" "$PROWLARR_CONTAINER" "$AUTOBRR_CONTAINER"; then
    for entry in "Sonarr|$SONARR_HOST_URL/ping" "Radarr|$RADARR_HOST_URL/ping" \
      "Prowlarr|$PROWLARR_HOST_URL/ping" "autobrr|$AUTOBRR_HOST_URL/"; do
      IFS='|' read -r app url <<<"$entry"
      ready=0
      for _ in $(seq 1 60); do
        code="$(curl -q -sS --noproxy "*" -o /dev/null -w '%{http_code}' "$url" || true)"
        if [[ "$code" == 200 || ( "$app" == autobrr && "$code" =~ ^(200|401|403)$ ) ]]; then
          ready=1
          break
        fi
        sleep 1
      done
      if (( ready == 0 )); then
        mark "certification TLS trust $app" "FAIL" "service did not become ready after trust update"
        status="FAIL"
      fi
    done
    mark "certification TLS trust" "PASS" "task CA installed only in Compose project $COMPOSE_PROJECT_NAME"
  else
    mark "certification TLS trust" "FAIL" "app containers are outside the declared certification Compose project"
    status="FAIL"
  fi
fi

SONARR_API_KEY="${SONARR_API_KEY_OVERRIDE:-$(api_key_from_container "$SONARR_CONTAINER")}"
RADARR_API_KEY="${RADARR_API_KEY_OVERRIDE:-$(api_key_from_container "$RADARR_CONTAINER")}"
PROWLARR_API_KEY="${PROWLARR_API_KEY_OVERRIDE:-$(api_key_from_container "$PROWLARR_CONTAINER")}"

configure_arr_client "Sonarr" "$SONARR_HOST_URL" "$SONARR_API_KEY" "/api/v3" "tvCategory" "tv-sonarr"
configure_arr_client "Radarr" "$RADARR_HOST_URL" "$RADARR_API_KEY" "/api/v3" "movieCategory" "radarr"
configure_arr_client "Prowlarr" "$PROWLARR_HOST_URL" "$PROWLARR_API_KEY" "/api/v1" "category" "prowlarr"
configure_autobrr_client

{
  echo
  echo "Overall status: $status"
} >> "$OUT"

echo "$OUT"
[[ "$status" == "PASS" ]]
