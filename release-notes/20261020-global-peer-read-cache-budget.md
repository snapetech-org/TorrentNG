---
category: changed
audience: operators
area: storage
action: none
breaking: false
---
Peer-read readahead now shares a process-wide backing-memory budget across torrents, and storage metrics report its usage and limit. Pressure eviction keeps accounting attached while a returned slice still uses the buffer. One exploratory seed pair measured CPU at 3.06 versus 3.29 CPU-s/GiB, while anonymous peak memory rose from 23.5 to 25.0 MiB; repeat runs and multi-torrent pressure tests are still needed, so no performance improvement is claimed.
