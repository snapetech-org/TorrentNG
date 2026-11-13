#!/usr/bin/env bash
set -euo pipefail

PROJECT="${1:?usage: install_certification_test_ca.sh COMPOSE_PROJECT CA_CERT CONTAINER...}"
CA_CERT="${2:?usage: install_certification_test_ca.sh COMPOSE_PROJECT CA_CERT CONTAINER...}"
shift 2
[[ -s "$CA_CERT" ]] || { echo "certification CA certificate is missing" >&2; exit 2; }
(( $# > 0 )) || { echo "at least one certification container is required" >&2; exit 2; }

for container in "$@"; do
  actual_project="$(docker inspect -f '{{ index .Config.Labels "com.docker.compose.project" }}' "$container" 2>/dev/null || true)"
  if [[ "$actual_project" != "$PROJECT" ]]; then
    echo "refusing to change TLS trust in $container (Compose project does not match $PROJECT)" >&2
    exit 1
  fi
  docker exec "$container" mkdir -p /usr/local/share/ca-certificates
  remote_cert=/usr/local/share/ca-certificates/torrentng-certification-test-ca.crt
  local_digest="$(sha256sum "$CA_CERT" | awk '{print $1}')"
  remote_digest="$(docker exec "$container" sh -lc "sha256sum '$remote_cert' 2>/dev/null | awk '{print \$1}'" 2>/dev/null || true)"
  if [[ "$local_digest" != "$remote_digest" ]]; then
    docker cp "$CA_CERT" "$container:$remote_cert" >/dev/null
    docker exec "$container" update-ca-certificates >/dev/null
    docker restart "$container" >/dev/null
  fi
done
