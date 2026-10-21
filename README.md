# TorrentNG

TorrentNG is a torrent manager for people who seed a lot. It ships as a native
desktop app, a headless server with a browser interface, and a bridge that puts
the same interface in front of a client you already run. All three use one
BitTorrent engine and one API.

It runs on your computer or server. There is no hosted TorrentNG account. The
files stay on the storage you configure.

![TorrentNG showing Debian, Ubuntu, and Fedora Linux downloads](docs/assets/torrentng-default-dark-linux-isos.png)

For local MCP access, see the [SnapeTech MCP integration guide](docs/integrations/snapetech-mcp.md).
## Is it useful for you?

TorrentNG may help if you manage torrents on a home server, NAS, or other
self-hosted machine and want to control them from one browser page. It can also
provide familiar automation APIs for supported clients.

If you already have a client and are happy with its interface, TorrentNG adds
something new to set up and maintain. If you just want a torrent app on your
own computer, install **TorrentNG Desktop**: it bundles the engine and needs no
server, Docker, or configuration.

## Choose what to install

TorrentNG has three parts. You normally need only one of them.

| You want to... | Install | What it is |
|---|---|---|
| Run torrents on this computer, with a normal app | **TorrentNG Desktop** | A native app (Linux, macOS, Windows) with the engine included. Low memory and CPU; no browser or server. It can also connect to a TorrentNG Server. |
| Run torrents on a home server, NAS, or seedbox and use it from a browser | **TorrentNG Server** (`torrentngd` + WebUI) | The headless engine with the browser interface and APIs. Available as Docker images, release binaries, and packages. |
| Keep qBittorrent, rTorrent, Transmission, or Deluge and add the TorrentNG interface | **TorrentNG WebUI Bridge** (`torrentng`) | A web interface and API for the client you already run. It replaces ruTorrent or the client's own WebUI and adds automation APIs, while your client keeps its files and runs transfers. |

Downloads are on the [releases page](https://github.com/snapetech-org/TorrentNG/releases):
`torrentng-desktop-*` for the Desktop app, `torrentngd-native-*` for the Server.
See the [Desktop guide](docs/DESKTOP.md) for install steps and what each file is.
The Server quick start is below. To connect an existing client, start with the
[WebUI Bridge deployment guide](docs/DEPLOYMENT.md). The parts use different services
and configuration.

## Try TorrentNG Server

You need Docker with Docker Compose v2 and OpenSSL. This starts the Engine on
your own machine and keeps its state and downloads in Docker volumes.

From the repository directory, create a private token file and start TorrentNG:

```sh
umask 077
printf 'TORRENTNG_API_TOKEN=%s\n' "$(openssl rand -hex 32)" > .env
docker compose --env-file .env -f deploy/native/compose.yml up --build
```

Open [http://localhost:28082](http://localhost:28082). At the login screen,
enter the value after `TORRENTNG_API_TOKEN=` from `.env` in either field. Keep
`.env` private; the same token is needed if you recreate the container. The
file is ignored by Git.

The WebUI is available only from this machine by default. Port `44444` is open
for BitTorrent peers. To let another person reach the WebUI, set up an
authenticated HTTPS reverse proxy first; see the
[Engine deployment guide](docs/NATIVE_DEPLOYMENT.md).

To stop the service, press Ctrl+C. The named volumes keep your downloads and
settings. **Do not add `-v` to `docker compose down` unless you intend to delete
those volumes.**

## Before using a real library

TorrentNG is pre-1.0. Settings and APIs may change. Back up your client state
and downloaded files before importing, moving, or deleting data. Migration
starts with a report; review it before using `--apply`. Some imported progress
may need a full data check before another client can resume it.

The dated [Debian ISO comparison and native memory investigation](certification/reports/native-memory-optimization-20261014-agentvm.md)
records public download, controlled seeding, and anonymous memory against
qBittorrent, Transmission, Deluge, and rTorrent. TorrentNG averaged 0.49 MiB/s
on its one public-swarm pass, slower than the completed peers at 5.60–16.82
MiB/s; in the controlled seed test it averaged 3.56 MiB/s, close to their
3.61–3.74 MiB/s. Its measured anonymous peaks were 8.3 MiB in public download
and 7.4 MiB while seeding. That was about 2.1–3.2 times less than qBittorrent
and Transmission, and 11–12 times less than Deluge. The peer ratios combine
sequential runs, with Transmission and Deluge measured in a later rerun; they
are not a general ranking. A separate system-allocator run used less memory
than the previous jemalloc run (18.8 MiB public / 25.3 MiB seed), while seed
rates were 3.56 vs 3.66 MiB/s. This allocator comparison is not a concurrent
A/B. A mimalloc follow-up used more anonymous memory (19.8 MiB public / 23.6
MiB seed) and did not complete the public download within 30 minutes. A
qBittorrent check right afterward completed in 39.6 seconds at 19.08 MiB/s,
which flags a current TorrentNG download gap; sequential testing leaves the
precise cause unresolved. The results do not show orders-of-magnitude savings
against every peer; rTorrent did not start, and public swarm speeds varied
sharply. The earlier
[unoptimized baseline](certification/reports/client-performance-benchmark-20261013.md)
is retained for reference. Neither report establishes a maximum torrent count.
See the dated [release evidence](docs/RELEASE_EVIDENCE.md) for what was
exercised and where.

## Learn more

- [Install TorrentNG Desktop](docs/DESKTOP.md)
- [Connect an existing client](docs/DEPLOYMENT.md)
- [Run the TorrentNG Engine](docs/NATIVE_DEPLOYMENT.md)
- [Install on Unraid](deploy/unraid/README.md)
- [Configuration](docs/CONFIGURATION.md)
- [OIDC sign-in setup and operations](docs/OIDC_AUTHENTICATION.md) · [shared provider catalog](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-provider-catalog.md)
- [Migration and export](docs/MIGRATION.md)
- [Client performance benchmark](docs/CLIENT_PERFORMANCE_BENCHMARK.md) · [dated comparison and memory investigation](certification/reports/native-memory-optimization-20261014-agentvm.md)
- [API and compatibility limits](docs/API.md) · [client matrices](docs/CLIENT_COMPATIBILITY_MATRICES.md)
- [Backup and restore](docs/BACKUP_RESTORE.md)
- [Documentation index](docs/README.md)
- [Localization roadmap](docs/localization-roadmap.md) — language coverage, regional number behavior, and remaining translation work

## Development

The source tree has separate workspaces for the built-in Engine and the
compatible-client service. The commands below are for contributors, not
required for the Docker quick start.

```sh
cargo build --release -p torrentngd
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development checks. The current
release qualification sequence and the hardware-dependent work to run later
are in the [at-home validation plan](docs/AT_HOME_VALIDATION.md).

## Support

Ask for setup help or report a problem on
[Discord](https://discord.gg/5PyXBfvS6T). For a security issue, follow
[SECURITY.md](SECURITY.md) and do not post it publicly.

If TorrentNG is useful to you, [support its development on Ko-fi](https://ko-fi.com/snapetech).

## Legal use

Users are responsible for the content they download, seed, or manage and for
complying with applicable laws and licenses.

## License

TorrentNG is available under `AGPL-3.0-or-later OR Commercial`. Unless you have
a separate signed commercial license, the AGPL applies. See [LICENSE](LICENSE)
and [COMMERCIAL-LICENSE.md](COMMERCIAL-LICENSE.md).

TorrentNG is not affiliated with or endorsed by qBittorrent, rTorrent,
Transmission, Deluge, or other projects named in this repository.

## Local security scanning

Run `./scripts/security-scan-local.sh` for local Semgrep CE and the Trivy scanners that fit this repository. The installed pre-commit hook runs Semgrep on staged source/config files only; Trivy runs on demand or in CI. This public repository also runs Semgrep CE in GitHub Actions on changes and weekly. See [security-scanning.md](docs/security-scanning.md). Use `./scripts/update-security-tools.sh` to refresh local scanner data and the Trivy database.
