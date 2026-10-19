#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: hardware_lane.sh ABSOLUTE_OUTPUT_DIR ABSOLUTE_STORAGE_ROOT" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
output_dir="$1"
storage_root="$2"
[[ "$output_dir" = /* && "$storage_root" = /* ]] || usage

repo_root="$(git rev-parse --show-toplevel)"
output_real="$(realpath -e -- "$output_dir")"
storage_real="$(realpath -e -- "$storage_root")"
case "$output_real/" in
  "$repo_root/"*)
    echo "hardware evidence must be outside the source checkout" >&2
    exit 2
    ;;
esac
[[ -d "$output_real" && ! -L "$output_dir" ]] || {
  echo "output directory must be an existing non-symlink directory" >&2
  exit 2
}
[[ -w "$output_real" ]] || {
  echo "output directory must be writable" >&2
  exit 2
}
[[ -d "$storage_real" && ! -L "$storage_root" ]] || {
  echo "storage root must be an existing non-symlink directory" >&2
  exit 2
}
case "$storage_real/" in
  "$repo_root/"*)
    echo "storage target must be outside the source checkout" >&2
    exit 2
    ;;
esac
case "$output_real/" in
  "$storage_real/"*)
    echo "output directory cannot be inside the storage target" >&2
    exit 2
    ;;
esac
case "$storage_real/" in
  "$output_real/"*)
    echo "storage target cannot be inside the output directory" >&2
    exit 2
    ;;
esac
[[ -w "$storage_real" ]] || {
  echo "storage root must be writable" >&2
  exit 2
}
[[ -z "$(find "$output_real" -mindepth 1 -maxdepth 1 -print -quit)" ]] || {
  echo "output directory must be empty" >&2
  exit 2
}
[[ -z "$(find "$storage_real" -mindepth 1 -maxdepth 1 -print -quit)" ]] || {
  echo "storage root must be empty and disposable" >&2
  exit 2
}

git -C "$repo_root" rev-parse HEAD > "$output_real/commit.txt"
git -C "$repo_root" status --porcelain=v1 --untracked-files=all > "$output_real/worktree.txt"
rustc -Vv > "$output_real/rustc.txt"
uname -a > "$output_real/uname.txt"
lscpu > "$output_real/lscpu.txt"
findmnt --target "$storage_real" --output SOURCE,FSTYPE,OPTIONS,TARGET \
  > "$output_real/storage-mount.txt"
stat -f -- "$storage_real" > "$output_real/storage-stat.txt"

set +e
cargo bench --locked -p rt-piece-picker --bench perf \
  > "$output_real/picker-bench.log" 2>&1
picker_status=$?
cargo bench --locked -p rt-api-native --bench perf \
  > "$output_real/api-snapshot-bench.log" 2>&1
snapshot_status=$?
TNG_STORAGE_MOVE_IMPORT_ROOT="$storage_real" \
  cargo test --locked -p rt-storage --test storage_move_import_hardware \
    -- --ignored --nocapture \
    > "$output_real/storage-move-import.log" 2>&1
storage_status=$?
set -e

cat > "$output_real/exit-status.txt" <<EOF
picker_bench=$picker_status
api_snapshot_bench=$snapshot_status
storage_move_import=$storage_status
EOF

if (( picker_status != 0 || snapshot_status != 0 || storage_status != 0 )); then
  echo "one or more dedicated hardware qualification commands failed" >&2
  exit 1
fi
