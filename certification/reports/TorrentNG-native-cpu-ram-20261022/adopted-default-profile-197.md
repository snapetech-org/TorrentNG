# Adopted default release-profile equivalence check

After the final 12/12 performance cycle, the repository's `[profile.release]` was changed from `opt-level = 3` to `opt-level = "s"`. This check verified the adopted default against the already built and tested opt-s candidate. It is a build equivalence check only; no tests or performance runs were executed.

- Source HEAD: `152be9714b4b229194877476023f4061784bc946`.
- Comparison base: `a8154943f9bd489d79e84ab13423befb576d2789`.
- Exact command (the override is explicitly unset): `unset CARGO_PROFILE_RELEASE_OPT_LEVEL; CARGO_TARGET_DIR=<target-dir> cargo +1.97.0 build --locked --release --bin torrentngd -j2`.
- Result: exit 0; Cargo reported `Finished release profile [optimized]` in 0.18 s. Wall time was 0.20 s. The existing isolated opt-s target was up to date.
- Rebuilt-target binary SHA-256: `4aaa587c0953a8a4d73a01c52ea4429cb03699f232d63c6c4fb0d2ef6b407e63`.
- Preserved candidate binary SHA-256: `4aaa587c0953a8a4d73a01c52ea4429cb03699f232d63c6c4fb0d2ef6b407e63`.
- Hashes match exactly, so the adopted default reproduces the tested opt-s candidate binary.
- ELF `.text` remains 12,670,727 bytes.

Production diff identities relative to the comparison base:

- Final production diff (`git diff --binary BASE -- crates Cargo.toml Cargo.lock`) SHA-256: `502fe6d88db3fbb07d4ae09a33cc94024aba250c809e4b014b199833b576b3ff`.
- The four-file runtime/rate source diff (network_budget.rs, transfer/rate.rs, torrentngd/src/main.rs, torrentngd/Cargo.toml) is unchanged from the consolidated gate, SHA-256 `c40762882b8f16e69994f85db813c91a9e7cbecfa77a819bb49f464cf564c9b2`.
- The only new production-profile change since that gate is root `Cargo.toml`: release opt-level is now `"s"`, with a comment noting opt-level 3 remains selectable using the environment override.

Build output is preserved in `adopted-default-profile-197-build.log`.
