---
category: fixed
audience: users, operators
area: network
action: none
breaking: false
---
TorrentNG keeps DHT announce tokens stable through a monotonic-clock regression, so peers can complete valid announcements instead of being rejected after a transient clock anomaly.
