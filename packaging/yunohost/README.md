# TorrentNG for YunoHost

This package provides two install-time modes:

- **TorrentNG engine and WebUI** runs the native `torrentngd` transfer engine.
- **WebUI/API for an existing torrent client** runs the `torrentng` sidecar and connects to an existing qBittorrent, Transmission, Deluge, rTorrent, or TorrentNG engine.

The package uses YunoHost packaging format 2. It supports YunoHost 12 on
`amd64` and `arm64`; its pinned bundle uses static musl binaries so it does not
depend on the host glibc version. The WebUI supports root and subpath installs,
and the package keeps its state separate from downloaded payloads.

This repository retains the package sources for existing YunoHost installs,
but no longer builds or publishes YunoHost bundles. The package manifest is
pinned to the previously published `yunohost-0.1.0-1` bundle. The app catalog
is maintained in a separate YunoHost repository.

## Local package check

Install the current package branch directly with YunoHost's package tooling:

```sh
sudo yunohost app install https://github.com/snapetech-org/torrentng_ynh --debug
```

The `tests.toml` file is intentionally minimal. The official package CI should
exercise install, backup, restore, upgrade, and removal against a YunoHost VM.
