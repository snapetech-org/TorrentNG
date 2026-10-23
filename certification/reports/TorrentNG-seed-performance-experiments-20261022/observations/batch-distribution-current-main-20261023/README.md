# Current upload-batch distribution — 2026-10-23

This is a **diagnostic count, not a performance result**. A 16 MiB local loopback seed transfer against the current main release binary emitted debug events for each storage read and Piece write. The transfer used `pread`, a 4 MiB/s receiver, 16 KiB blocks, a 32-request window, and fixture SHA-256 `8720b45a392e63940e529552c5cb8be01e422e0fc58b538302c952f19c37f012`. The binary SHA-256 was `844479163668b69bfe378838540dd9973fb60b3a781c237a2eefbd0d33d2fd07`.

| Operation | 1 frame/block | 7 frames/blocks | 8 frames/blocks | Total operations |
|---|---:|---:|---:|---:|
| Storage read batches | 1 | 1 | 127 | 129 |
| Ready Piece-frame groups | 1 | 1 | 127 | 129 |
| Vectored socket writes | 1 | 1 | 127 | 129 |

All 1,024 Piece frames were verified. Apart from the initial singleton and final seven-block remainder, the current path already batches eight blocks for this workload. That makes a broader collection delay a weak next CPU hypothesis for this traffic. Debug logging was enabled, so this run has no CPU or memory score. The sanitized counts and identities are in [`summary.json`](summary.json); the per-client debug log and generated credentials were excluded.

No production change was made. The result only describes this 16 MiB, one-peer, paced loopback diagnostic; other peer request patterns may batch differently.
