#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
release_workflow="$repo_root/.github/workflows/release.yml"
publisher="$repo_root/scripts/promote_dockerhub_tags.sh"

require_text() {
  file="$1"
  text="$2"
  if ! grep -F -- "$text" "$file" >/dev/null; then
    printf 'Docker tag policy check failed: missing %s in %s\n' "$text" "${file#"$repo_root"/}" >&2
    exit 1
  fi
}

require_text "$publisher" 'latest main master stable'
require_text "$publisher" 'skopeo list-tags docker://docker.io/snapetech/torrentng-native'
require_text "$publisher" 'skopeo list-tags docker://docker.io/snapetech/torrentng'
require_text "$publisher" 'if [ "$ci_tag" != "$latest_common_stable_tag" ]; then'
require_text "$publisher" 'skopeo copy "docker://${NATIVE_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng-native:${stable_alias}"'
require_text "$publisher" 'skopeo copy "docker://${PACKAGED_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng:${stable_alias}"'

require_text "$release_workflow" 'group: dockerhub-stable-aliases'
require_text "$release_workflow" 'run: sh scripts/check_docker_tag_sync.sh'
require_text "$release_workflow" 'stable_tag_pattern='
require_text "$release_workflow" 'git ls-remote --tags origin'
require_text "$release_workflow" 'sort -V'
require_text "$release_workflow" 'if [[ "$RELEASE_TAG" =~ $stable_tag_pattern && "$RELEASE_TAG" == "$latest_stable_tag" ]]; then'
require_text "$release_workflow" 'for tag in "$RELEASE_TAG" "$version"; do'
require_text "$release_workflow" 'for stable_alias in latest main master stable; do'
require_text "$release_workflow" 'docker tag "$local_image:$RELEASE_TAG" "$ghcr_image:$stable_alias"'
require_text "$release_workflow" 'docker tag "$local_image:$RELEASE_TAG" "$dockerhub_image:$stable_alias"'
require_text "$release_workflow" 'publish_image torrentng-native ghcr.io/snapetech/torrentng/native snapetech/torrentng-native'
require_text "$release_workflow" 'publish_image torrentng ghcr.io/snapetech/torrentng/sidecar snapetech/torrentng-sidecar snapetech/torrentng'
require_text "$release_workflow" 'publish_image torrentng-phase1 ghcr.io/snapetech/torrentng/phase1 snapetech/torrentng-phase1'

if grep -Eq 'for tag in .* (latest|main|master|stable)' "$release_workflow"; then
  echo 'Docker tag policy check failed: moving aliases must only be pushed through the latest-stable gate.' >&2
  exit 1
fi

printf 'Docker stable tag policy is present: latest, main, master, and stable share the latest stable release.\n'
