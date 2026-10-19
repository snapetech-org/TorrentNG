# PERF-111 initial local diagnostic run (historical)

Date: 2026-10-18
Source checkout: `codex/transfer-perf-runner-20261018`, HEAD
`4cd671b0d5eae1ddc096493ded381569e57f49f4`, with the rebuild worktree dirty.
Host: `kspld0`, Linux 7.1.7 x86_64.
Toolchain: `rustc 1.94.0`.

These are single-host diagnostic samples from the optimized Cargo bench
profile, rerun after the v2 completion-projection guard was added. The source
was the dirty rebuild worktree identified above, so these numbers are
historical and are superseded for ticket acceptance by two clean exact-main
`eb5f3158` runs on `kspls0`, recorded in
[`TRANSFER_CORE_ACTIVE_SPECS.md`](TRANSFER_CORE_ACTIVE_SPECS.md). Neither set
is a before/after comparison or whole-client efficiency claim. No timing
thresholds were applied.

Commands:

```text
cargo bench -p rt-piece-picker --bench perf
cargo bench -p rt-api-native --bench perf
```

## Piece picker

The picker probe performs 100,000 pick/cancel operations per size. Availability
rows apply 20 add/remove cycles for the named bitfield shape.

```csv
probe,piece_count,iterations,total_ns,ns_per_operation,picker_bytes
picker_pick_cancel,1000,100000,7953480,79,245416
availability_add_remove_empty,1000,20,4539,226,16256
availability_add_remove_sparse_1_in_64,1000,20,11863,593,16356
availability_add_remove_dense,1000,20,302494,15124,22380
picker_pick_cancel,12000,100000,9061723,90,2382880
availability_add_remove_empty,12000,20,57830,2891,193632
availability_add_remove_sparse_1_in_64,12000,20,115008,5750,194380
availability_add_remove_dense,12000,20,2791146,139557,242764
picker_pick_cancel,100000,100000,137684629,1376,5795616
availability_add_remove_empty,100000,20,252148,12607,1612632
availability_add_remove_sparse_1_in_64,100000,20,634254,31712,1620796
availability_add_remove_dense,100000,20,22349792,1117489,2136892
```

## Native API snapshots

The unchanged-snapshot probe takes 1,000 cached samples. Changed-row timings
are three separate one-row samples per catalog size. Allocation columns are
cumulative allocation traffic recorded by the probe, not retained heap or RSS.

```csv
probe,torrent_count,samples,total_ns,ns_per_sample,allocation_calls,allocated_bytes
snapshot_initial,1000,1,5238679,5238679,36791,4725704
snapshot_cached_unchanged,1000,1000,99539,99,1,64
snapshot_one_row_changed,1000,1,116180,116180,71,54103
snapshot_one_row_changed,1000,1,133483,133483,81,55454
snapshot_one_row_changed,1000,1,116812,116812,71,54213
snapshot_initial,10000,1,63103249,63103249,291210,45249568
snapshot_cached_unchanged,10000,1000,109368,109,1,64
snapshot_one_row_changed,10000,1,214377,214377,79,161111
snapshot_one_row_changed,10000,1,307303,307303,93,163598
snapshot_one_row_changed,10000,1,169732,169732,79,161221
snapshot_initial,100000,1,656011833,656011833,2662198,417861120
snapshot_cached_unchanged,100000,1000,99028,99,1,64
snapshot_one_row_changed,100000,1,284931,284931,167,188911
snapshot_one_row_changed,100000,1,304508,304508,225,203894
snapshot_one_row_changed,100000,1,386213,386213,167,189021
```

## Qualification still needed

PERF-111 probe acceptance is complete on the exact source, with two retained
dedicated-host runs documented in the active ticket spec. Matched whole-client
resource and goodput observations remain separate qualification gates before
any efficiency or client-comparison claim.
