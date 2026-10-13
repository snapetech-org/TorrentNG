//! TorrentNG Desktop: a native, low-footprint client for `torrentngd`.
//!
//! Everything except `app`/`ui` is plain logic that is unit-tested without a
//! display; the egui layer is deliberately thin.

pub mod api;
pub mod app;
pub mod backend;
pub mod config;
pub mod fmt;
pub mod model;
pub mod sync;
pub mod theme;
pub mod ui;
pub mod view;
