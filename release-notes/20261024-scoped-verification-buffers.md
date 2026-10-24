### Performance

- Add an opt-in Linux verification scratch allocator (`TNG_STORAGE_ANON_LARGE_FRAMES=1`). It reuses at most two large anonymous buffers while verification runs and releases idle mappings when the last verification scope ends. Allocations remain charged to the frame cap; ordinary allocations outside verification and non-Linux platforms keep their existing backing.
- Retain the experiment behind an explicit setting while qualifying transfer CPU, PSS, recheck CPU and mixed workloads. This change does not establish rTorrent parity or change buffered torrent file I/O defaults.
