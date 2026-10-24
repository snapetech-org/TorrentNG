# Reproduction settings

The runs used the local controlled-seed mode, a private task-local tracker,
the same pinned fixture within each size, source-built rTorrent/libtorrent
0.16.26, and a qBittorrent 5.2.3 leecher. No public swarm or metadata lookup
was used.

Common settings:

```sh
TNG_CLIENT_BENCH_MODE=controlled-seed
TNG_CLIENT_BENCH_COUNTER_MODE=perf
TNG_CLIENT_BENCH_RATE_BPS=1073741824
TNG_CLIENT_BENCH_SEED_LEECHER_RATE_BPS=-1
TNG_CLIENT_BENCH_SAMPLE_SECS=0.5
TNG_CLIENT_BENCH_PROCESS_TREE_SAMPLE_SECS=2
TNG_CLIENT_BENCH_CPUSET=18-19
TNG_CLIENT_BENCH_SEED_LEECHER_CPUSET=20-21
```

The 1 GiB fixture was `/var/tmp/torrentng-perf-counters-20261024/fixture.bin`
with SHA-256
`342049a2faa17d05512e1089cc0810046568876cf7de1b1133afad2afa65fccc`.
The 4 GiB fixture was
`/var/tmp/torrentng-perf-counters-20261024/r8-nowait-crosscheck/fixture-4g.bin`
with SHA-256
`826ccedc7d79ec09eb8aa5e35eed560e0113fec36024a63f650785f845bbb9b4`.

The 1 GiB pair selected `torrentngd,rtorrent`, used image
`torrentng/torrentngd:benchmark-20261024t211127z`, and source fingerprint
`a92373574c5d01c9030a8449b00a081d3905a5f01e9b5545914a3b093daf206e`.
NOWAIT was disabled for `nowait-off-1g-r2` and enabled for
`nowait-on-1g-r2` with `TNG_CLIENT_BENCH_STORAGE_PEER_READ_NOWAIT=1`.

The 4 GiB TorrentNG pair selected only `torrentngd`, used image
`torrentng/torrentngd:benchmark-20261024t212138z`, and source fingerprint
`2c90a33ad485658a0a1fd4f533671ddd31f60919554080e3f5521890597b7d03`.
NOWAIT was disabled for `nowait-off-4g-r3` and enabled for
`nowait-on-4g-r3`. The separate 4 GiB rTorrent run selected only `rtorrent`
under the same benchmark harness and fixture.

Both image labels were checked against their source commit, fingerprint, and
allocator. BuildKit reused the cached release binary; NOWAIT is a runtime
environment option and did not change the compiled artifact. The benchmark
attached `perf stat` before adding the torrent, recorded instructions, user
cycles, context switches, and CPU migrations, and normalized only after the
leecher file passed SHA-256. The counter window includes announce/startup
overhead. Host load and all raw samples are in each run report.

The first long temporary path exceeded Linux's rTorrent SCGI socket path limit;
that attempt stopped during preflight. An early 4 GiB run also timed out while
waiting for a tracker `incomplete` count after it reported `2 complete, 0
incomplete`. The harness now proceeds to the live peer/status loop in that
state and still requires the final payload SHA-256. Both failed attempts are
preserved under [excluded evidence](excluded/).
