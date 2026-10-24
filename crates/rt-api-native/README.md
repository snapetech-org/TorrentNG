# rt-api-native

TorrentNG REST and WebSocket API (axum).

The crate name is retained as a source/package compatibility label; the API is
the shared TorrentNG surface used by both `torrentngd` and the compatible-client
service.

## Status: Implemented — TorrentNG client support

## State ownership

`AppState` is an `Arc`-backed handle so route handlers and middleware share one
configuration allocation. Cloning the handle does not clone credentials or every
store handle. Field reads retain their existing syntax through `Deref`; replacing
a configuration field uses `Arc::make_mut`, preserving clone-local configuration
changes. Existing lock-backed registry, caches, metrics and mutable API stores
remain shared across clones, including after a configuration copy-on-write.
Construct state through `AppState` constructors rather than struct literals.
