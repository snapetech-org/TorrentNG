#!/bin/sh
set -eu

ci_tag="${CI_COMMIT_TAG:-}"
: "${CI_COMMIT_SHORT_SHA:?CI_COMMIT_SHORT_SHA is required}"
: "${NATIVE_IMAGE_SHA_TAG:?NATIVE_IMAGE_SHA_TAG is required}"
: "${PACKAGED_IMAGE_SHA_TAG:?PACKAGED_IMAGE_SHA_TAG is required}"

skopeo copy "docker://${NATIVE_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng-native:${CI_COMMIT_SHORT_SHA}"
skopeo copy "docker://${PACKAGED_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng:${CI_COMMIT_SHORT_SHA}"

if [ -z "$ci_tag" ]; then
  exit 0
fi

# Keep the immutable release tag even when an older release pipeline is rerun.
skopeo copy "docker://${NATIVE_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng-native:${ci_tag}"
skopeo copy "docker://${PACKAGED_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng:${ci_tag}"

stable_tag_pattern='^main-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]-[0-9]+$'
if ! printf '%s\n' "$ci_tag" | grep -Eq "$stable_tag_pattern"; then
  printf 'Skipping stable aliases for non-stable tag %s.\n' "$ci_tag"
  exit 0
fi

extract_stable_tags() {
  awk '
    /"Tags"[[:space:]]*:/ { in_tags = 1 }
    in_tags {
      line = $0
      while (match(line, /"[^"]+"/)) {
        tag = substr(line, RSTART + 1, RLENGTH - 2)
        if (tag ~ /^main-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]-[0-9]+$/) print tag
        line = substr(line, RSTART + RLENGTH)
      }
      if (/\]/) exit
    }
  '
}

native_tags="$(skopeo list-tags docker://docker.io/snapetech/torrentng-native | extract_stable_tags)"
packaged_tags="$(skopeo list-tags docker://docker.io/snapetech/torrentng | extract_stable_tags)"
latest_common_stable_tag="$(
  printf '%s\n' "$native_tags" | awk -v packaged_tags="$packaged_tags" '
    BEGIN {
      count = split(packaged_tags, packaged, "\n")
      for (i = 1; i <= count; i++) packaged_map[packaged[i]] = 1
    }
    !packaged_map[$0] { next }
    {
      count = split($0, release, "-")
      release_date = release[2] + 0
      release_number = release[3] + 0
      if (release_date > latest_date || (release_date == latest_date && release_number > latest_number)) {
        latest = $0
        latest_date = release_date
        latest_number = release_number
      }
    }
    END { print latest }
  '
)"

if [ -z "$latest_common_stable_tag" ]; then
  echo "No common stable release image is present in both Docker Hub repositories." >&2
  exit 1
fi

if [ "$ci_tag" != "$latest_common_stable_tag" ]; then
  printf 'Skipping stable aliases for %s; latest stable release is %s.\n' \
    "$ci_tag" "$latest_common_stable_tag"
  exit 0
fi

for stable_alias in latest main master stable; do
  skopeo copy "docker://${NATIVE_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng-native:${stable_alias}"
  skopeo copy "docker://${PACKAGED_IMAGE_SHA_TAG}" "docker://docker.io/snapetech/torrentng:${stable_alias}"
done
