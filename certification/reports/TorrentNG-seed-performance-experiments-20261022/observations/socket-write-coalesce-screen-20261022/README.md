# Opportunistic eight-frame socket-write screen — 2026-10-22

This screen tested a narrower alternative to increasing storage-read batch
size: after one four-block read completed, poll once for a second TCP read
batch that was already ready and combine both completions in one vectored
write. It did not wait for the second read, change the four-block/64 KiB
storage bound, or change the 16 ms coalescing deadline. A second variant also
raised the ready-request intake cap from four to eight.

Both variants were isolated from production source commit
`0849da9cc4f795c23f607bccd47a0ec5eb0fc1e5`, built with Rust 1.94.0, and screened
on the same verified 16 MiB local fixture (SHA-256
`132646868d8a9abf88d5fe16244ec101009f6b76613437514dbd41769d3943d3`). The
4 MiB/s transfer window was 4.0001 seconds. `RUST_LOG=torrentng::upload_batches=debug`
was enabled for event counts, so these one-run CPU/PSS/anonymous readings are
diagnostic only, not performance scores.

## Results

| Variant | Storage reads | TCP write calls and frame groups | CPU seconds | Peak PSS | Peak anonymous |
|---|---:|---|---:|---:|---:|
| Current source | 257 (255×4, 1×3, 1×1) | 257 (255×4, 1×3, 1×1) | 0.08035 | 18.18 MiB | 5.11 MiB |
| Ready-only second completion | 256 (256×4) | 254 (252×4, 2×8) | 0.08738 | 18.50 MiB | 5.95 MiB |
| Intake cap 8 plus ready-only merge | 402 (200×4, 6×3, 10×2, 186×1) | 280 (117×5, 76×4, 69×1, 7×2, 6×3, 3×6, 2×8) | 0.09429 | 18.61 MiB | 5.94 MiB |

The first variant combined only two pairs of reads and saved three socket
write calls (`257 → 254`, 1.2%). Its single diagnostic run also read 0.08738
CPU seconds versus 0.08035 for the baseline. The tiny sample is too short to
call that a stable CPU regression, but the mechanism saved too few calls to
justify a larger comparison.

Raising intake to eight fragmented storage work: single-block reads increased
from one to 186 and total storage reads from 257 to 402. Socket writes rose to
280. Its one diagnostic run also used more CPU, PSS, and anonymous memory than
the baseline. This variant is stopped.

## Decision and reproducibility

Neither patch changed production source. Keep the current 16 ms behavior and
four-block storage batches. The ready-only and intake-eight patches are
retained under [`experiments/`](experiments/), and the raw summaries and
client traces are in [`runs/`](runs/). The baseline and both candidates
completed and passed the same SHA-256 check. The parent
[`manifest.json`](../manifest.json) hashes the retained evidence files.
