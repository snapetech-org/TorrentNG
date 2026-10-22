# Consolidated Rust 1.97 build and correctness gate

## Source and toolchain

- Source checkout HEAD: `152be9714b4b229194877476023f4061784bc946`.
- Source comparison base: `a8154943f9bd489d79e84ab13423befb576d2789`.
- Full production diff (`git diff --binary BASE -- crates Cargo.toml Cargo.lock`) SHA-256: `5f7cf1e84b4a11cb7f15e48f8b495d065fcab07746e9ade3e3ee2f08cfdf4bad`.
- Main four-file rate/runtime diff (network_budget.rs, transfer/rate.rs, torrentngd/src/main.rs, torrentngd/Cargo.toml) SHA-256: `c40762882b8f16e69994f85db813c91a9e7cbecfa77a819bb49f464cf564c9b2`.
- Retained dense verify source diff SHA-256: `eebbcaa4010b7bac4e4f0ee55871505e3d3843384c47634dbb613cc44c9d5c26`.
- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, host `x86_64-unknown-linux-gnu`, LLVM 22.1.6; Cargo 1.97.0 (`c980f4866 2026-06-30`). System allocator.
- Both builds used locked dependencies, thin LTO, one codegen unit, stripped debuginfo. Normal profile used opt-level 3. Compact profile used `CARGO_PROFILE_RELEASE_OPT_LEVEL=s`; no repository Cargo profile change.

## Release builds

| Profile | Command | Isolated target | Result | Elapsed |
|---|---|---|---|---:|
| Normal | `cargo +1.97.0 build --locked --release --bin torrentngd -j2` | `<target-dir>` | exit 0 | 5m 38s |
| Compact | `CARGO_PROFILE_RELEASE_OPT_LEVEL=s cargo +1.97.0 build --locked --release --bin torrentngd -j2` | `<target-dir>` | exit 0 | 4m 30s |

Binaries were copied out immediately after both builds and before any feature-specific test build:

| Artifact | SHA-256 | File bytes | ELF `.text` bytes |
|---|---|---:|---:|
| `bin/torrentngd-consolidated-opt3-197` | `3078a818a3b65007f7741b26a284e22205ba07996916f56b05ce5cec68ce8fea` | 29,070,400 | 18,320,887 |
| `bin/torrentngd-consolidated-size-197` | `4aaa587c0953a8a4d73a01c52ea4429cb03699f232d63c6c4fb0d2ef6b407e63` | 27,685,088 | 12,670,727 |
| Prior verify-197 comparison binary | `94a24269b7aefb191be39d7d955c9e25cf93bdacd3914bd6d36756569a356cd0` | 31,302,968 | 19,781,879 |

ELF `.text` sizes are from `readelf -SW`, section `.text`; they are not GNU `size`'s aggregate `text` column.

## Tests

All commands used Rust 1.97.0, `--locked --release`, and `-j2`; all exited 0.

| Command target | Result |
|---|---|
| `-p rt-engine --lib` (normal opt-3 target) | 769 passed, 0 failed, 0 ignored; suite 6.58 s |
| `-p torrentngd --bin torrentngd` (normal opt-3 target) | 32 passed, 0 failed, 1 ignored |
| `-p torrentngd --bin torrentngd --features full-compat` (normal opt-3 target) | 32 passed, 0 failed, 1 ignored |
| `-p torrentngd --bin torrentngd` (compact opt-s target) | 32 passed, 0 failed, 1 ignored |

The sole ignored daemon test in each configuration is `tests::panic_hook_child_entrypoint`, an intentionally ignored subprocess entrypoint for the panic-hook regression.

## Clippy

Clippy component was added to the Rust 1.97.0 toolchain after the initial check found it absent. All three release Clippy commands exited 0 with `-D warnings`:

- `cargo +1.97.0 clippy --locked --release -p rt-engine --lib -j2 -- -D warnings`
- `cargo +1.97.0 clippy --locked --release -p torrentngd --bin torrentngd -j2 -- -D warnings`
- `cargo +1.97.0 clippy --locked --release -p torrentngd --bin torrentngd --features full-compat -j2 -- -D warnings`

Logs: `consolidated-clippy-engine.log`, `consolidated-clippy-daemon-default.log`, and `consolidated-clippy-daemon-full-compat.log`. Test logs persisted for full-compat and compact configurations as `consolidated-full-compat-tests.log` and `consolidated-size-profile-tests.log`. The engine and default daemon test counts were captured from the successful command output.

No performance measurements were run during this gate.
