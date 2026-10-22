# Mapped-window read probe — 2026-10-22

This small Linux probe refines the earlier whole-file mmap comparison. It
compared a reusable 64 KiB `pread` buffer with mmap windows sized to one 64 KiB
upload batch and one 1 MiB piece-sized window. Each 64 KiB batch represents
four 16 KiB peer blocks. The fixture was the 64 MiB file with SHA-256
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

Each of three samples processed 256 MiB per method. All checksums matched.
Median process CPU was:

| Method | CPU-s/GiB median | Change vs. 64 KiB `pread` |
|---|---:|---:|
| Reused-buffer `pread`, 64 KiB per call | 0.31291 | baseline |
| mmap and unmap a 64 KiB window per batch | 0.29131 | −6.9% |
| mmap and unmap a 1 MiB window per piece | 0.28607 | −8.6% |

The earlier whole-file map probe reported a 56.3% reduction against 16 KiB
`pread` calls. This smaller gain uses the current engine's already-batched
64 KiB read size and includes mapping and unmapping per batch or piece. It
suggests that a bounded mapped-window design may save a little read-stage CPU,
but the difference is small and cannot be projected onto the whole client.

This is a hot-file data-access microprobe. It excludes TorrentNG's scheduler,
descriptor admission, socket framing, rate control, cancellation, cold-file
faults, and memory accounting; it did not measure PSS. The 1 MiB mapping ran
third in each pair, so this is an exploratory screen with possible order
bias. It does not justify adding mmap to the production path. A cached mapping
could amortize map syscalls further, but would need a global bound, protection
against file truncation, worker-safe page faults, and accounting for mapped
pages before it could be measured in the whole-client probe.

The reproduction source and raw sample output are in this directory. Compile
with `gcc -O3 -Wall -Wextra -Werror mmap_window_probe.c -o mmap_window_probe`,
then pass any local 64 MiB file that has been warmed into the page cache. The
exact test file and SHA-256 are recorded in `results.txt`.
