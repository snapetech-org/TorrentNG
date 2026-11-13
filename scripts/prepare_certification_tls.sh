#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${1:?usage: prepare_certification_tls.sh OUTPUT_DIR}"
HOSTNAME="${2:-torrentng-cert-tls}"

if [[ -e "$OUT_DIR/ca.crt" || -e "$OUT_DIR/server.crt" || -e "$OUT_DIR/server.key" ]]; then
  echo "refusing to overwrite existing certification TLS material in $OUT_DIR" >&2
  exit 2
fi
command -v openssl >/dev/null || { echo "openssl is required" >&2; exit 2; }

umask 077
mkdir -p "$OUT_DIR"
chmod 0700 "$OUT_DIR"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/torrentng-cert-tls.XXXXXX")"
cleanup() {
  rm -rf -- "$WORK_DIR"
}
trap cleanup EXIT

openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 2 \
  -keyout "$WORK_DIR/ca.key" -out "$WORK_DIR/ca.crt" \
  -subj "/CN=TorrentNG Certification Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -addext "subjectKeyIdentifier=hash" >/dev/null 2>&1
openssl req -new -newkey rsa:2048 -nodes -sha256 \
  -keyout "$WORK_DIR/server.key" -out "$WORK_DIR/server.csr" \
  -subj "/CN=$HOSTNAME" >/dev/null 2>&1
cat >"$WORK_DIR/server.ext" <<EXTENSIONS
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:$HOSTNAME,DNS:localhost,IP:127.0.0.1
EXTENSIONS
openssl x509 -req -sha256 -days 2 -in "$WORK_DIR/server.csr" \
  -CA "$WORK_DIR/ca.crt" -CAkey "$WORK_DIR/ca.key" -CAcreateserial \
  -extfile "$WORK_DIR/server.ext" -out "$WORK_DIR/server.crt" >/dev/null 2>&1

install -m 0644 "$WORK_DIR/ca.crt" "$OUT_DIR/ca.crt"
install -m 0644 "$WORK_DIR/server.crt" "$OUT_DIR/server.crt"
install -m 0600 "$WORK_DIR/server.key" "$OUT_DIR/server.key"
printf 'certification TLS material prepared in %s\n' "$OUT_DIR"
