---
category: security
audience: users, operators
area: security
action: none
breaking: false
---
TorrentNG updates vulnerable Rust and WebUI dependencies and upgrades the bundled rTorrent runtime to 0.16.28 to address known security issues. The native and sidecar lockfiles pin `quinn-udp` to 0.5.16, and the native, compatible-client, and Phase 1 images pin Alpine zlib to 1.4-r0.
