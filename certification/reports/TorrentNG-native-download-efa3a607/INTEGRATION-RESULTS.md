# Native download patch integration

Integrated `torrentng-native-download.patch` from the supplied ZIP. The package targets `efa3a607`; current `main` was `e7a39c12`, whose intervening changes from that base were limited to OIDC files. The patch applied cleanly, and the six resulting engine/picker source files match the package's recorded SHA-256 values.

## Changes

- `TransferJobs` continues polling actor-owned I/O while inline writes, file preparation, and durability barriers wait. Completed results remain owned by the actor and count against its existing 64-job limit until consumed.
- Piece selection prefers an eligible admitted partial piece among equally rare choices, while rechecking peer availability, priority, sequential policy, and state limits. Hybrid selection filters pieces reserved by another connection before picking; ordinary v1 peers can still share pieces.

## Validation on the integrated source

- `cargo fmt --all -- --check`: passed.
- `cargo test --locked -p rt-engine -p rt-piece-picker --lib`: **847 passed** (767 engine, 80 picker).
- `cargo build --release --locked -p torrentngd`: passed with Rust 1.94.0.
- Local 64-piece transfer: passed; all 64 pieces verified and payload SHA-256 was `e0e370e98b3c4935ff0b20a077054c7fe2c904c43f4d3020dab7c47aaedcbc0c`.
- Local 756-piece transfer, three attempts: two completed and one reset. Both successful runs transferred all 756 pieces (792,723,456 bytes) and matched SHA-256 `d9006b23b6f88d7a407f6583788a9c368448eea18e3622add9a20f2948c98241`. The first uninstrumented run reset at 63.5% (480 pieces verified); the next instrumented run and a second uninstrumented retry completed. The reset cause remains unknown.

The local peer test validates correctness only. It is not a CPU/RAM comparison. The prior qBittorrent controlled-download stall has not yet been confirmed fixed; see the subsequent matched client comparison report when available.

Raw request traces, daemon logs, and result JSON are retained in `validation/integrated-*`; downloaded payload files were kept outside the repository.
