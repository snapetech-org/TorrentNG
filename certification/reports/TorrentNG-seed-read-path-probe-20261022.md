# TorrentNG seed read-path microprobe — 2026-10-22

## Result

A small Linux probe compared a reused 16 KiB `pread` buffer with a single
read-only mapping over a hot 64 MiB file. It touched every byte in each block
and produced matching checksums. Three paired runs used alternating order;
each sample processed 256 MiB.

| Paired run | `pread` CPU-s/GiB | `mmap` CPU-s/GiB | `mmap` change |
|---:|---:|---:|---:|
| 1 | 0.1064 | 0.0469 | -55.9% |
| 2 | 0.1019 | 0.0465 | -54.4% |
| 3 | 0.1086 | 0.0414 | -61.9% |
| **Median** | **0.1064** | **0.0465** | **-56.3%** |

The checksum matched for every pair. The median mapped read path used about
2.29 times less CPU in this probe, a difference of 0.0599 CPU-s/GiB. The
2026-10-22 whole-client screen had a 0.78 CPU-s/GiB active-window gap, so this
isolated read-stage difference is only about 8% of that gap as a scale
comparison. It cannot close the CPU gap alone. A mapped-range engine prototype
is worth considering only if it also removes repeated scheduler work or
buffer ownership cost; this result does not predict a whole-client gain.

## Reproduction

On `kspld0`, the probe compiled with GCC 16.2.1 using:

```sh
cc -O3 -march=native -Wall -Wextra -Werror \
  scripts/seed_read_path_probe.c -o /tmp/seed-read-path-probe
/tmp/seed-read-path-probe /tmp/seed-read-path-fixture.bin
```

The probe creates a new deterministic 64 MiB fixture, warms both paths before
timing, runs three paired samples in alternating order, prints raw CPU and wall
times plus checksums, then removes the fixture. Each method processes 768 MiB
across the three samples.

## Limits and next step

This is a data-access microprobe. It does not include TorrentNG's scheduler,
descriptor admission, socket writes, rate accounting, cancellation, cold-file
faults, or concurrent file mutation. The mapping is established once and stays
resident for the timed reads. The result is evidence to prototype only the
mapped data source while retaining TorrentNG's existing Piece framing and
vectored socket writer.

Modern rTorrent/libtorrent `v0.16.25` maps file ranges into `MemoryChunk`s and
keeps an upload chunk while subsequent requests remain in the same piece. Its
plain upload path writes from mapped chunk memory; encryption takes a separate
buffered path. See upstream [`FileList::create_chunk_part` and
`create_chunk`](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/torrent/data/file_list.cc)
and [`PeerConnectionBase::load_up_chunk` and upload write path](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/protocol/peer_connection_base.cc).

If pursued, an integrated candidate should be limited to Linux, verified v1
data, plaintext TCP, and a request contained in one real file range. It must
reduce scheduler or buffer-ownership work as well as use mapped data, while
preserving descriptor and storage identity, bounded ownership, file-lifecycle
safety, cancellation, request ordering, fairness, and the buffered fallback.
The current benchmark also observed a 63-second gap between rTorrent's first
connected peer and first payload, without recording peer interest/choke/request
state; diagnose that before interpreting startup-inclusive service results.
The latest local screen and its limits are recorded in the
[current-source comparison report](TorrentNG-current-rtorrent-screen-20261022.md).
