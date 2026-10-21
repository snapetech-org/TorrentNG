# TorrentNG native download fixes — efa3a607 review

This package contains new native-engine changes against `efa3a607e0ea93399529343dab6b231162ebfaef`. The earlier upload/storage architecture package is already implemented in that base. Do not reapply the earlier ZIP.

Final local commit: `c7b586a0aa8d990c658fbcfa775791d95897ff5c`, following `5127d18bf1ea191e46a3c14dc9e63587fbacf7a7`, on `codex/native-download-efa3a607`.

## Findings and changes

### 1. Confirmed download I/O deadlock

The torrent actor kept download write/hash futures in `FuturesUnordered` and polled them from its main event loop. The same actor could call `save_fastresume` and await a storage checkpoint inline. A previously polled write can still own the storage write-admission read guard while waiting for lane capacity, a file open, a device permit, or a backend result. The checkpoint needs the exclusive guard. Awaiting the checkpoint stops the actor from polling the write that must release it. A backend finishing does not advance the suspended Rust future. Inline file preparation and spill writes have the equivalent dependency on file locks or storage capacity.

The included regression parks a real `MountScheduler::write_at` after it acquires the checkpoint fence, releases lane capacity, and attempts a checkpoint. On the original implementation, the checkpoint times out. The patched implementation completes the write and the real durability barrier.

`TransferJobs` now polls pending transfer I/O while an inline operation is waiting. It retains completed results for the normal actor path, preserving generation checks and avoiding recursive completion/checkpoint handling. Pending and completed results both count toward the existing 64-job admission bound. Payload leases remain owned through completion consumption. This adds no Tokio task per block and does not remove fsync or weaken resume trust.

Confidence: **high** that the deadlock exists and this regression is fixed. Its connection to the saved qBittorrent download stall is **unconfirmed**. That run stopped around its first checkpoint, which is consistent with this mechanism, but the available log is insufficient to identify its exact wait. The original daemon completed local custom-peer transfers; those successes do not disprove the deterministic deadlock.

### 2. Random selection happened per block instead of per new piece

The picker selected a random equally rare piece for each 16 KiB request. A local 756-piece transfer spread its first 64 requests over 63 pieces. Another partial run had requested all 756 pieces and received 140 MiB without producing one verified HAVE. This exercised hundreds of partial states, assembly eviction/spill writes, and disk verification before useful piece completion.

The picker now continues admitted partial pieces among equally rare choices. New pieces still use random tie breaking; rarer pieces, explicit priorities, enabled files, and sequential policy retain their precedence. A common continuation needs one small hint, not a second torrent-sized index. After a peer or policy change, fallback searches bounded partial state. Webseeds also continue partial pieces. New-state admission is checked before choosing a candidate so a full state budget does not hide existing usable partial work.

This changes the source of work: fewer concurrent assemblies and ledger states, more contiguous requests, and more opportunities to hash and write complete pieces. It does not prove a numeric whole-client CPU/RAM improvement.

### 3. Preserve hybrid peer ownership during selection

Hybrid transfers reserve a whole piece to one connection. Continuing a partial piece without applying that rule could repeatedly select work owned by another connection. A borrowed availability view now filters that work before useful-window calculation and picking. It preserves packed bitmap iteration and creates no filtered bitmap per request. The authoritative reservation check remains. Ordinary v1 transfers still allow multiple peers to supply one piece.

## Scope

These are the live Rust v1/hybrid transfer actor, its ownership ledger adapter, and the shared piece picker. No sidecar, WebUI, comparator configuration, or benchmark scoring change is included. The separate pure-v2 actor's scheduler is unchanged. Its existing tests still run in the engine suite.

The supplied seed comparison remains the current comparison evidence: TorrentNG 1.33 CPU-s/GiB and 16.9 MiB anonymous RAM. This package targets download liveness and download work; it does not claim to close the remaining seed gap to Transmission or rTorrent.

## Validation

- Original code plus the checkpoint regression: **expected failure**, checkpoint timeout. See `validation/regression-before.log` and the separate `checkpoint-regression-only.patch`.
- Final engine suite: **767 passed**; piece-picker suite: **80 passed**. This includes real checkpoint I/O, file-lock and lane-capacity dependencies, cancelled inline waiting with retained payload leases, rarity/priority rules, disjoint peers, full picker state budget, and hybrid ownership.
- Format and whitespace checks passed.
- Final daemon build passed. The native daemon completed a **756 MiB download**, announced all 756 verified pieces, reached 100% progress, and the completed file matched SHA-256 `d9006b23b6f88d7a407f6583788a9c368448eea18e3622add9a20f2948c98241`. The first 64 requests covered one piece. There were 48,398 requests for 48,384 distinct blocks; this is not a claim of zero duplicate traffic.
- Binary SHA-256 and source commits are recorded in `validation.json`. The native transfer uses the daemon's actual peer socket with unique deterministic data per block; the API is used only to configure the torrent and read status. It is correctness evidence, not a resource comparison.

Existing and new tests cover the relevant invariants; no comparator matrix or public swarm run was launched. The original qBittorrent reproduction still needs to be rerun to confirm whether this resolves that specific stall.

## Apply and reproduce

Use a clean checkout at the base above, or review applicability to newer main before applying:

```sh
git apply --check torrentng-native-download.patch
git apply torrentng-native-download.patch
cargo test --locked -p rt-engine -p rt-piece-picker --lib
cargo build --release --locked -p torrentngd
python3 reproduce/native_download_check.py \
  --binary target/release/torrentngd --output /tmp/tng-native-download-check \
  --pieces 756 --rate 33554432 --timeout 45
```

The output directory must not exist. Adjust the transfer deadline for a slower machine. The script does not record CPU/RAM. It shuts down its own daemon and leaves its output directory for inspection. The validation here used Rust 1.94.0, Linux GNU, release LTO disabled and 16 codegen units to reuse the local development build; production uses its normal build profile.

To reproduce the original deadlock, use a separate clean base checkout, apply only `checkpoint-regression-only.patch`, and run:

```sh
cargo test --locked -p rt-engine --lib checkpoint_drives_pending_write_before_waiting_for_its_fence
```

Do not apply that test-only patch on top of the complete patch; the complete patch already includes the test.

`source/` contains all six final changed files. The complete patch includes implementation and tests, with no dependency changes. `SHA256SUMS` covers every package file. Commits are local to the isolated review branch; nothing in this package has been pushed to remote main.
