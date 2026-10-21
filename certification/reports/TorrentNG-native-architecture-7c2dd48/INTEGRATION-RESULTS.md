# Native architecture integration results

Integrated the architecture changes into the local checkout at `7c2dd481ca224da8f5e47b783678b6110146ba9b`, which already contained the prior native batching/cache-retirement patch.

The archive checksum is `191e94a691a9071dd0280dd6c20669fd4166921cc9b86183ef9f78218ee3d167`. The 29 archive members passed CRC verification, and all 28 `SHA256SUMS` entries verified. After integration, all 13 files under the archive's `source/` directory matched byte-for-byte.

## Integrated changes

- Native cache hits can validate the original file through a weak descriptor reference and bypass device-slot admission, descriptor catalog lookup, LRU bookkeeping, and an extra operation-handle allocation.
- Contended cache misses suspend as bounded continuations instead of occupying a disk worker while waiting for device admission. Descriptor ownership is deferred until the request resumes.
- Piece mapping borrows immutable metadata, validates a request before exposing regions, and carries file ordinals into upload spans. Upload and download paths avoid the prior per-block region allocation, repeated path cloning/resolution, and linear file lookup.
- Bounded upload collections use inline storage; a single-block upload retains its lease directly instead of allocating shared ownership. Multi-block payloads still use shared ownership.
- HDD elevator tasks and channels are created only when an eligible read uses that route. Resource-error labels are formatted only on error.

The archive's incremental patch initially did not apply directly to this checkout because the prior cache code had already been rustfmt-formatted. Its official patch check passed on an isolated index after applying the earlier ZIP patch. I then applied the exact delta from the current files to the archive's checksum-verified source snapshots. Existing sidecar/WebUI edits were left untouched.

## Validation performed here

- `cargo test --quiet --locked -j4 -p rt-piece-map -p rt-storage -p rt-engine --lib -- --test-threads=1`: **1,140 passed** (762 engine, 29 piece-map, 349 storage).
- `python3 scripts/test_client_performance_benchmark.py`: **45 passed**.
- `cargo build --release --locked -j4 -p torrentngd`: passed with LTO disabled and 16 codegen units; build finished in 2m07s.
- The built daemon passed two complete 8 MiB wire transfers through the real peer protocol: request windows 1 and 32, 512 blocks each. Both payloads matched SHA-256 `13cda4de78e9de4ebe14230e819a87cf0f50999b91d94152da76e0d2fef66513`. The tested binary SHA-256 was `e47cf0a103f9bcf9b5e06192bf7cca9be00a54253e20e2bf49fee5de8c7d5415`.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

The current-run test/build logs and both wire-result JSON files are preserved under [validation/integration](validation/integration/).

These checks establish source integration, unit correctness, and wire correctness. They do not measure CPU or RAM and do not establish superiority over qBittorrent, Transmission, or Deluge. The implementation's remaining per-read worker handoff is documented in [README.md](README.md).
