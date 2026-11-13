# rTorrent tinyxml2 Build Profile

This directory contains the Phase 1 build helper for a pinned current stable
rTorrent engine.

| Variable | Default |
|---|---|
| `RTORRENT_REF` | `v0.16.28` |
| `LIBTORRENT_REF` | `v0.16.28` |
| `PREFIX` | `/usr/local` |
| `WORKDIR` | `/tmp/torrentng-build` |

Run on a host with build dependencies installed:

```sh
./engine-profile/build/build-rtorrent-tinyxml2.sh
```

The Phase 1 image uses the same `--with-xmlrpc-tinyxml2` option. Build inputs
are fetched by immutable commit in the Dockerfiles. TorrentNG carries small
source patches for tracker identity control, bounded multicall RPCs, startup
identity ordering, and unlimited queue refill.

The rTorrent 0.16.28 source handles nonblocking SCGI accepts and a full task
ring upstream. The deterministic regression compiles the actual upstream
handler from the pinned checkout with socket fixtures for every cursor and
free-slot position:

```sh
python3 engine-profile/build/check-scgi-accept-regression.py --source /path/to/v0.16.28/src/rpc/scgi.cc
```

The started-view range handler shares the result of a sole
`scheduler.simple.update=` command within each batch. That command scans the
complete started list, so repeating the global pass for every row causes
excessive startup work when some downloads cannot open. Other views, mixed
commands, and per-download mutations retain their per-row behavior. The native
regression compiles the actual patched range handler: 1,037 rows require two
global passes across two pages, with identical response shapes and scheduling
results.

```sh
python3 engine-profile/build/check-scheduler-range-regression.py --source /path/to/v0.16.28/src/command_events.cc
```
