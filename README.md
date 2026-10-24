# TorrentNG

TorrentNG is a self-hosted torrent manager with a native desktop app, a
headless server and browser interface, and a bridge for clients you already
run. Your torrents and files stay on the storage you configure; TorrentNG has
no hosted account.

![TorrentNG showing Linux downloads](docs/assets/torrentng-default-dark-linux-isos.png)

## Table of contents

- [What it is](#what-it-is)
- [Choose what to install](#choose-what-to-install)
- [Start the server](#start-the-server)
- [Project and performance status](#project-and-performance-status)
- [Research and results](#research-and-results)
- [Before using a real library](#before-using-a-real-library)
- [Development](#development)
- [Support and license](#support-and-license)

## What it is

TorrentNG provides one WebUI and automation API for its built-in Engine and
supported connections to qBittorrent, rTorrent, Transmission, and Deluge.
With an existing client, that client keeps control of transfers and files.
With the TorrentNG Engine, TorrentNG manages transfers, storage, and saved
state. See the [local MCP integration guide](docs/integrations/snapetech-mcp.md)
for local assistant access.

The Engine supports [Swarm Switch](docs/SWARM_SWITCH.md) in the WebUI: compare
public and private swarm observations, use a compatible public counterpart with
the same files, and restore the retained private torrent after verification.

## Choose what to install

You normally need one of these:

| You want to... | Install | What it does |
|---|---|---|
| Run torrents on this computer as a normal app | **TorrentNG Desktop** | Native app for Linux, macOS, and Windows. It bundles the engine and needs no server or Docker. |
| Run torrents on a home server, NAS, or seedbox | **TorrentNG Server** | Headless native engine with WebUI and APIs. Available as Docker images, release binaries, and packages. |
| Keep your existing torrent client | **TorrentNG WebUI Bridge** | Adds the TorrentNG interface and automation API while the existing client continues to manage transfers and files. |

Downloads are on the [releases page](https://github.com/snapetech-org/TorrentNG/releases).
Find installation details in the [Desktop guide](docs/DESKTOP.md),
[native Engine deployment guide](docs/NATIVE_DEPLOYMENT.md), and
[WebUI Bridge deployment guide](docs/DEPLOYMENT.md).

## Start the server

You need Docker with Docker Compose v2 and OpenSSL. From the repository
directory, create a private API token and start the service:

    umask 077
    printf 'TORRENTNG_API_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
    docker compose --env-file .env -f deploy/native/compose.yml up --build

Open [http://localhost:28082](http://localhost:28082). At the login screen,
enter the value after TORRENTNG_API_TOKEN= from .env in either field. Keep
.env private; the same token is needed if you recreate the container.

The WebUI is local-only by default. Port 44444 is open for BitTorrent peers.
To expose the WebUI, configure authenticated HTTPS first; see the
[Engine deployment guide](docs/NATIVE_DEPLOYMENT.md). Press Ctrl+C to stop.
Named volumes keep your downloads and settings. Do not add -v to
docker compose down unless you intend to delete those volumes.

## Project and performance status

TorrentNG is pre-1.0; settings and APIs can change. The latest comparison
against qBittorrent, Transmission, Deluge, and rTorrent used three randomized
rounds of local, hash-verified seeding. All 15 transfers passed. The current
build is still above rTorrent on CPU and cgroup anonymous memory. It used less
anonymous memory than Transmission and Deluge, and was near qBittorrent.

| Client / build | CPU seconds/GiB, whole window ↓ | CPU seconds/GiB, active payload ↓ | Peak cgroup anonymous MiB ↓ |
|---|---|---:|---:|---:|
| TorrentNG 304a45e (main-equivalent release defaults) | 4.17 | 3.18 | 7.2 |
| [qBittorrent 5.3.0](https://www.qbittorrent.org/download) | 2.04 | 2.33* | 6.4 |
| [Transmission 4.1.3](https://github.com/transmission/transmission/releases/) | 2.09 | 1.42 | 17.5 |
| [Deluge 2.2.0](https://pypi.org/project/deluge/) | 3.22 | 3.22 | 82.8 |
| [rTorrent 0.16.26](https://github.com/rakshasa/rtorrent/releases/latest) | 1.12 | 0.85 | 3.1 |

These are medians from three rounds. *qBittorrent active CPU was available in
two rounds. TorrentNG/rTorrent process-tree PSS medians were 16.5/18.7 MiB,
but PSS was unavailable for the other clients and was sampled less often.
Anonymous memory is the comparable all-client RAM measure here. The host had
sustained I/O pressure, so these are screening results, not a broad ranking or
parity qualification.

See the [full results and per-round evidence](certification/reports/TorrentNG-current-client-comparison-20261024/README.md),
[exact run configuration](certification/reports/TorrentNG-current-client-comparison-20261024/run-config.md),
and [benchmark Compose file](deploy/interop/compose.yml).

The best fully repeated optimized candidate is the 2026-10-23 scoped ordered
PGO buffered build: 3.409 CPU-s/GiB and 11.525 MiB peak PSS versus rTorrent at
2.439 and 10.533. Its paired ratios were 1.480 for CPU and 1.092 for PSS; it
failed the joint parity gate. The [frozen candidate config](certification/reports/TorrentNG-seed-execution-20261023/selected-candidate.json),
[qualification findings](certification/reports/TorrentNG-seed-execution-20261023/README.md),
and [Astra review](docs/reviews/2026-10-23-astra-architecture-review.md)
preserve its details. PGO and mapped backing remain opt-in.

## Research and results

- [Current comparison summary](docs/research/current-client-comparison.md) ·
  [full report and 15 verified runs](certification/reports/TorrentNG-current-client-comparison-20261024/README.md)
- [Exact comparison configuration and reproduction notes](certification/reports/TorrentNG-current-client-comparison-20261024/run-config.md)
- [Linux NOWAIT seed-read experiment](certification/reports/TorrentNG-nowait-seed-screen-20261024/README.md) — promising one-pass CPU/RAM reductions; remains opt-in and still above rTorrent.
- [Performance research index](docs/research/README.md) — best measured
  configurations, exact configs, discussion, gains, losses, and reverts
- [Historical performance ledger](docs/research/performance-history.md)
- [Archived detailed benchmark results](docs/research/client-performance-benchmark-archive.md)
- [Benchmark how-to](docs/CLIENT_PERFORMANCE_BENCHMARK.md)
- [Astra architecture review](docs/reviews/2026-10-23-astra-architecture-review.md)
- [PR #233 native-engine performance work](https://github.com/snapetech-org/TorrentNG/pull/233) ·
  [PR #235 comparison tooling and evidence](https://github.com/snapetech-org/TorrentNG/pull/235)
- [Backend performance claims audit](docs/BACKEND_PERFORMANCE_BENCHMARKS.md) ·
  [performance-scale claims audit](docs/PERFORMANCE_SCALE_CLAIMS_AUDIT.md)
- [Documentation index](docs/README.md)

## Before using a real library

Back up client state and downloaded files before importing, moving, or deleting
data. Migration starts with a report; review it before using --apply. Imported
progress may need a full data check before another client can resume it. See
the [migration guide](docs/MIGRATION.md) and
[backup and restore guide](docs/BACKUP_RESTORE.md).

## Development

Build the native server with Cargo:

    cargo build --release -p torrentngd

The default server build includes the native and qBittorrent APIs. Use the
full-compat feature to enable Transmission or Deluge RPC in a direct
torrentngd deployment. See [configuration](docs/CONFIGURATION.md) and
[contributing](CONTRIBUTING.md) for details.

Run the [local security scan](docs/security-scanning.md) with
scripts/security-scan-local.sh. The project also runs Semgrep in GitHub
Actions.

## Support and license

Ask for setup help or report a problem on
[Discord](https://discord.gg/5PyXBfvS6T). For security issues, follow
[SECURITY.md](SECURITY.md) and do not post them publicly.

Users are responsible for the content they download, seed, or manage and for
following applicable laws and licenses.

If TorrentNG is useful to you, [support its development on Ko-fi](https://ko-fi.com/snapetech).

TorrentNG is available under AGPL-3.0-or-later OR Commercial. Unless you have
a separate signed commercial license, the AGPL applies. See [LICENSE](LICENSE)
and [COMMERCIAL-LICENSE.md](COMMERCIAL-LICENSE.md).

TorrentNG is not affiliated with or endorsed by qBittorrent, rTorrent,
Transmission, Deluge, or other projects named in this repository.
