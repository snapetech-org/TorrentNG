# rt-api-qbit

qBittorrent Web API v2 compatibility facade over TorrentNG-client state.

## Status: Implemented — compatibility facade

This crate exposes both `/api/qb/v2` and `/api/v2` routes so automation tools
can configure TorrentNG as a qBittorrent-compatible download client.

The facade projects TorrentNG-client registry and engine metadata into qBit
response shapes. Compatibility structs are intentionally not the internal engine
model.

Covered surfaces include auth, app info, torrent list/add/control/delete,
trackers, files, categories, tags, transfer info, `sync/maindata`, RSS/search
probe endpoints, and common qBit v5 aliases.

Run focused tests:

```sh
cargo test -p rt-api-qbit
```

Run the full TorrentNG-client compatibility gate:

```sh
scripts/native_engine_certification_report.sh
```

## State ownership

`AppState` is an `Arc`-backed handle so route handlers and middleware share one
configuration allocation. Cloning the handle does not clone credentials or every
store handle. Field reads retain their existing syntax through `Deref`; replacing
a configuration field uses `Arc::make_mut`, preserving clone-local configuration
changes. Existing lock-backed registry, caches, metrics and mutable API stores
remain shared across clones, including after a configuration copy-on-write.
Construct state through `AppState` constructors rather than struct literals.
