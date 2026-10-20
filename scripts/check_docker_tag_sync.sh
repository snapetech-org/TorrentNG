#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
ci_file="$repo_root/.gitlab-ci.yml"
publisher="$repo_root/scripts/promote_dockerhub_tags.sh"

require_text() {
  file="$1"
  text="$2"
  if ! grep -F -- "$text" "$file" >/dev/null; then
    printf 'Docker tag policy check failed: missing %s in %s\n' "$text" "${file#"$repo_root"/}" >&2
    exit 1
  fi
}

require_text "$ci_file" 'dockerhub:tag-policy:'
require_text "$ci_file" 'sh scripts/check_docker_tag_sync.sh'
require_text "$ci_file" 'sh scripts/promote_dockerhub_tags.sh'
require_text "$ci_file" 'resource_group: dockerhub-stable-aliases'

require_text "$publisher" 'latest main master stable'
require_text "$publisher" 'skopeo list-tags docker://docker.io/snapetech/torrentng-native'
require_text "$publisher" 'skopeo list-tags docker://docker.io/snapetech/torrentng'
require_text "$publisher" 'if [ "$ci_tag" != "$latest_common_stable_tag" ]; then'
require_text "$publisher" 'skopeo copy "docker://${NATIVE_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng-native:${stable_alias}"'
require_text "$publisher" 'skopeo copy "docker://${PACKAGED_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng:${stable_alias}"'

printf 'Docker stable tag policy is present: latest, main, master, and stable share the latest stable release.\n'
