#!/usr/bin/env bash
set -euo pipefail

images=(
  ghcr.io/snapetech/torrentng/native
  ghcr.io/snapetech/torrentng/sidecar
)
aliases=(latest main master stable)

digest_for() {
  local image="$1" output digest
  if ! output="$(docker buildx imagetools inspect "$image" 2>&1)"; then
    printf 'Could not inspect %s:\n%s\n' "$image" "$output" >&2
    return 1
  fi
  digest="$(awk '$1 == "Digest:" { print $2; exit }' <<< "$output")"
  if [[ ! "$digest" =~ ^sha256:[a-f0-9]{64}$ ]]; then
    printf 'No valid manifest digest in inspect output for %s:\n%s\n' "$image" "$output" >&2
    return 1
  fi
  printf '%s\n' "$digest"
}

for image in "${images[@]}"; do
  source_ref="${image}:latest"
  expected_digest="$(digest_for "$source_ref")"
  pinned_source="${image}@${expected_digest}"

  tag_args=()
  for alias in "${aliases[@]}"; do
    target_ref="${image}:${alias}"
    if [[ "$target_ref" != "$source_ref" ]]; then
      tag_args+=(--tag "$target_ref")
    fi
  done

  if ((${#tag_args[@]})); then
    if ! create_output="$(docker buildx imagetools create --prefer-index=false "${tag_args[@]}" "$pinned_source" 2>&1)"; then
      printf 'Could not copy %s to legacy aliases:\n%s\n' "$pinned_source" "$create_output" >&2
      exit 1
    fi
    if [[ -n "$create_output" ]]; then
      printf '%s\n' "$create_output"
    fi
  fi

  for alias in "${aliases[@]}"; do
    target_ref="${image}:${alias}"
    actual_digest=''
    for attempt in {1..10}; do
      actual_digest="$(digest_for "$target_ref" 2>/dev/null || true)"
      if [[ "$actual_digest" == "$expected_digest" ]]; then
        break
      fi
      printf 'Waiting for %s to report %s (attempt %s/10).\n' "$target_ref" "$expected_digest" "$attempt"
      sleep 2
    done
    if [[ "$actual_digest" != "$expected_digest" ]]; then
      printf '%s points to %s; expected %s.\n' "$target_ref" "$actual_digest" "$expected_digest" >&2
      exit 1
    fi
  done
  printf 'Verified legacy aliases for %s (%s).\n' "$source_ref" "$expected_digest"
done
