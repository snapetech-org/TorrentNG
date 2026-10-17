---
category: changed
audience: users, operators
area: engine
action: none
breaking: false
---
TorrentNG's native engine now defaults to the system allocator, which used less anonymous memory than the prior jemalloc build in Debian ISO tests. Public swarm download performance remains under investigation; alternative allocators remain opt-in.
