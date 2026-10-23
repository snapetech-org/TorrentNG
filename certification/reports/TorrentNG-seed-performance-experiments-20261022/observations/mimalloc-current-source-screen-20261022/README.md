# Current-source mimalloc seed screen — 2026-10-22

This was a preliminary allocator check against the current product source.
The system-allocator release binary
(`63e316c8727c00e3c3356f48076ee7afeb4654b6050435f1c037c1c30247df99`) was
built from `main` at `0849da9cc4f795c23f607bccd47a0ec5eb0fc1e5`; the
`mimalloc-allocator` build
(`a055e741dc7fc16840af3851b67077b8ba0fd922cd20a2258abf80ca5ba44b59`) was
built from checkout `128c403b5bdd135b4fe071da929a52db5fb41328`. That checkout
differs from the system build only in the CodeQL workflow and its policy test;
the product sources and `Cargo.lock` match. Both used Rust 1.94.0, the same
16 MiB local fixture, and a 4 MiB/s paced receiver.
The fixture SHA-256 was
`132646868d8a9abf88d5fe16244ec101009f6b76613437514dbd41769d3943d3`.

Each row is one randomized pair and both clients completed SHA-256
verification. The short 4-second windows make CPU estimates noisy; this is a
screen, not a performance qualification or an rTorrent comparison.

| Setting | Client | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| Default purge delay | System allocator | 7.369 | 18.29 MiB | 5.68 MiB |
| Default purge delay | mimalloc | 7.358 | 22.82 MiB | 10.10 MiB |
| `MIMALLOC_PURGE_DELAY=0` | System allocator | 6.054 | 18.19 MiB | 5.59 MiB |
| `MIMALLOC_PURGE_DELAY=0` | mimalloc | 7.890 | 23.26 MiB | 10.35 MiB |

Default mimalloc was effectively CPU-tied with its paired system build, while
using 4.53 MiB more PSS and 4.42 MiB more anonymous memory. Zero-delay purge
did not recover that memory and used more CPU in its pair. The two system
control observations varied substantially in CPU, so the candidate CPU
difference is not stable; the repeated direction of the memory difference is
enough to stop this allocator screen before a larger run.

No allocator default or source code changed. The exact probe summaries are
retained under [`runs/`](runs/); generated payloads, session databases, and
temporary credentials are excluded. The parent
[`manifest.json`](../manifest.json) hashes each retained evidence file.
