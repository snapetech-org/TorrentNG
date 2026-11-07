# rTorrent tinyxml2 Build Profile

This directory contains the Phase 1 build helper for a pinned rTorrent engine.

Defaults:

| Variable | Default |
|---|---|
| `RTORRENT_REF` | `v0.16.11` |
| `LIBTORRENT_REF` | `v0.16.11` |
| `PREFIX` | `/usr/local` |
| `WORKDIR` | `/tmp/torrentng-build` |

Run on a host with build dependencies installed:

```sh
./engine-profile/build/build-rtorrent-tinyxml2.sh
```

The Phase 1 Dockerfile uses the same flags:

```sh
./engine-profile/build/build-rtorrent-tinyxml2.sh
```

Important configure flag:

```sh
--with-xmlrpc-tinyxml2
```

That selects tinyxml2 XMLRPC handling for the known-good bundle. The host helper mirrors the Docker build flow: run `autoreconf -fi`, build matching rTorrent/libtorrent tags, and apply the TorrentNG user-agent XMLRPC patch when available.

The packaged SCGI accept patch must normalize a failed wrapped search to the
full task-list end. Returning the wrapped range's end as a task can reopen a
busy event and abort the backend under saturation. The deterministic native
regression compiles the actual patched `event_read` from the checksum-verified
upstream source, with socket fixtures for every cursor and free-slot position:

```sh
python3 engine-profile/build/check-scgi-accept-regression.py --source /path/to/pinned/src/rpc/scgi.cc
```

The former patch reproduces the already-open exception in 63 of 72 cases;
the corrected patch passes all 72. This checks task selection and rejected
connection accounting; real network and full runtime qualification remain
separate gates.
