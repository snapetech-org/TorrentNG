#!/bin/sh
set -eu

repo_root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/torrentng-tag-promotion.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT HUP INT TERM

mkdir -p "$scratch/bin"
cat > "$scratch/bin/skopeo" <<'STUB'
#!/bin/sh
set -eu
case "$1" in
  copy)
    printf '%s\n' "$3" >> "$SKOPEO_COPY_LOG"
    ;;
  list-tags)
    case "$2" in
      docker://docker.io/snapetech/torrentng-native|docker://docker.io/snapetech/torrentng)
        printf '%s\n' '{"Tags":["main-20261022-20","v20261023-1"]}'
        ;;
      *)
        printf 'Unexpected registry query: %s\n' "$2" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    printf 'Unexpected skopeo command: %s\n' "$1" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$scratch/bin/skopeo"

PATH="$scratch/bin:$PATH" \
SKOPEO_COPY_LOG="$scratch/copies.log" \
CI_COMMIT_TAG=v20261023-1 \
CI_COMMIT_SHORT_SHA=0123456 \
NATIVE_IMAGE_SHA_TAG=localhost/native:test \
PACKAGED_IMAGE_SHA_TAG=localhost/packaged:test \
  sh "$repo_root/scripts/promote_dockerhub_tags.sh"

if ! grep -Fxq 'docker://docker.io/snapetech/torrentng-native:latest' "$scratch/copies.log"; then
  echo 'The newest date-based release did not receive the stable Docker Hub alias.' >&2
  exit 1
fi

if ! grep -Fxq 'docker://docker.io/snapetech/torrentng:stable' "$scratch/copies.log"; then
  echo 'The newest date-based release did not receive the stable Docker Hub package alias.' >&2
  exit 1
fi

echo 'Docker Hub stable aliases select the newest date-based release across legacy and v tags.'
