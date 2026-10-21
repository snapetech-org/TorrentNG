---
category: changed
audience: operators
area: storage
action: none
breaking: false
---
Peer-read caches across torrents now share a process-wide backing-memory cap, limiting memory retained by concurrent read-ahead. Metrics show live use and the cap, and buffers remain charged while returned data still references them.
