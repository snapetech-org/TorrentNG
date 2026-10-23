# Upload-obligation lock reduction screen — 2026-10-23

**Decision: not promoted.** The candidate carried the choke-priority decision out of dequeue, avoiding duplicate peer-state checks while composing each upload read batch. The focused state-machine tests passed and all transfers verified, but the short whole-client screen showed no joint CPU/PSS gain. The production source remains unchanged.

## Result

Three randomized 16 MiB pairs used a local single-file seed at 4 MiB/s, 16 KiB requests, a 32-request window, `pread`, and loopback only (no DHT, PEX, or tracker). Fixture SHA-256: `132646868d8a9abf88d5fe16244ec101009f6b76613437514dbd41769d3943d3`. All six transfers completed and verified.

| Build | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current source | 3.648 | 18.208 MiB | 5.516 MiB |
| Lock-reduction candidate | 3.861 (+5.8%) | 18.466 MiB (+1.4%) | 5.531 MiB (+0.3%) |

Paired CPU changes were -1.9%, +7.8%, +12.6%. CPU was higher in two pairs. Host load fell during the screen (1-minute load average 39.27 to 28.43); CPU variance makes the median inconclusive as a regression. Since the screen showed neither a stable CPU improvement nor a memory win, no longer screen was spent on this candidate.

## Candidate and checks

The patch replaces repeated `peek_request`, choke/allowed-fast queries, and a second dequeue with one state-locked dequeue that returns `(block, choke_critical)`. A non-batchable request is returned to the queue front as before. Choke eligibility, per-frame finish/cancellation, rate admission, and storage reads retain their existing owners.

- Focused `rt-engine` obligation tests: 13 passed.
- Formatting and optimized daemon build: passed.
- All six full-client transfers passed the same SHA-256 check.
- Control binary SHA-256: `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`.
- Candidate binary SHA-256: `e2293fcdee98de47db7ae4d76c895d420636e96191509f1efcba1674b3d8f2ae`.
- Candidate patch SHA-256: `09201958bd7176b8929283ca98426125655c373f3cf16f3764b9e29731778253` (gzip SHA-256 `8c55fe009d96b6e9224a7159b92b3dc14c06fa419172e7aeb90ed3accdf4aec6`).
- Exact sanitized per-round metrics: [`summary.json`](summary.json); commands and validation: [`validation.txt`](validation.txt).

This is a local TorrentNG-to-TorrentNG screen only; it includes no rTorrent run and establishes no comparator parity. The compact evidence omits temporary session databases, API tokens, bootstrap credentials, and client logs.
