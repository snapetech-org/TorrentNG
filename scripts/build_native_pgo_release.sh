#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build_native_pgo_release.sh PROFILE.profdata[.gz] [TARGET_DIR] [SOURCE_REPO] [FUNCTION_ORDER]

Build torrentngd with LLVM profile-guided optimization using the pinned Rust
toolchain and the workspace's normal release profile. This experimental build
does not change Cargo.toml release defaults. The output hash identifies this
build; clean builds have not been byte-for-byte reproducible.
SOURCE_REPO can select an older clean baseline without copying scripts into it.
FUNCTION_ORDER optionally selects a source/compiler/PGO-matched order generated
by native_seed_function_order.py. It needs its manifest sidecar. This remains
experimental and requires independent matched CPU/PSS qualification.
EOF
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 1 || $# -gt 4 ]]; then
  usage >&2
  exit 2
fi

if [[ -n ${RUSTFLAGS:-} || -n ${CARGO_ENCODED_RUSTFLAGS+x} ]]; then
  echo "unset RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS to use the pinned PGO flags" >&2
  exit 2
fi

if [[ -n ${RUSTC:-} || -n ${RUSTC_WRAPPER:-} || -n ${RUSTC_WORKSPACE_WRAPPER:-} || -n ${CARGO_BUILD_RUSTC:-} || -n ${CARGO_BUILD_RUSTFLAGS:-} ]]; then
  echo "unset compiler and Cargo build overrides to use the pinned Rust toolchain" >&2
  exit 2
fi

while IFS='=' read -r env_name _; do
  case "$env_name" in
    CARGO_PROFILE_*)
      echo "unset $env_name to use the workspace release profile" >&2
      exit 2
      ;;
  esac
done < <(env)

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source_repo=$(realpath "${3:-$repo_root}")
profile_input=$(realpath "$1")
function_order=""
if [[ -n ${4:-} ]]; then
  function_order=$(realpath "$4")
fi
target_input=${2:-"$repo_root/target/pgo-release"}
mkdir -p "$target_input"
target_dir=$(cd "$target_input" && pwd)
profile_data="$target_dir/torrentng-seed-profile.profdata"

if [[ ! -f $profile_input ]]; then
  echo "profile file not found: $profile_input" >&2
  exit 2
fi

case "$profile_input" in
  *.gz)
    gzip -t "$profile_input"
    gzip -dc "$profile_input" > "$profile_data"
    ;;
  *)
    cp "$profile_input" "$profile_data"
    ;;
esac

if ! rustc +1.97.0 -V >/dev/null 2>&1; then
  echo "Rust toolchain 1.97.0 is required" >&2
  exit 2
fi

echo "PGO profile SHA-256: $(sha256sum "$profile_data" | cut -d ' ' -f 1)"
echo "Target directory: $target_dir"
cd "$source_repo"
python3 "$repo_root/scripts/native_seed_pgo.py" validate --profile "$profile_input" --repo "$source_repo"
if [[ -n $function_order ]]; then
  python3 "$repo_root/scripts/native_seed_function_order.py" validate \
    --order "$function_order" --repo "$source_repo" --pgo-profile "$profile_input"
  RUSTFLAGS="-Cprofile-use=$profile_data" \
    cargo +1.97.0 rustc --locked --release -p torrentngd --bin torrentngd --target-dir "$target_dir" -- \
      -Clink-arg=-Wl,--emit-relocs "-Clink-arg=-Wl,--symbol-ordering-file=$function_order"
else
  RUSTFLAGS="-Cprofile-use=$profile_data" \
    cargo +1.97.0 build --locked --release -p torrentngd --target-dir "$target_dir"
fi

echo "torrentngd SHA-256: $(sha256sum "$target_dir/release/torrentngd" | cut -d ' ' -f 1)"

RUSTFLAGS="-Cprofile-use=$profile_data" python3 "$repo_root/scripts/seed_efficiency_tools.py" manifest \
  --binary "$target_dir/release/torrentngd" --repo "$source_repo" --profile "$profile_data" \
  --output "$target_dir/release/torrentngd.manifest.json"

if [[ -n $function_order ]]; then
  python3 "$repo_root/scripts/native_seed_function_order.py" validate \
    --order "$function_order" --repo "$source_repo" --pgo-profile "$profile_input" \
    --build-manifest "$target_dir/release/torrentngd.manifest.json"
fi
