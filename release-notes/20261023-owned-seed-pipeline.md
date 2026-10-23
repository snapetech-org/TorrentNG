---
category: changed
audience: operators
area: performance
action: Regenerate experimental PGO profiles with scripts/native_seed_pgo.py train before using the PGO build script.
breaking: false
---
Complete seeds release download-selection storage, upload rate waits avoid retaining payload, and interrupted TCP uploads preserve rate accounting for bytes already sent. Linux mapped uploads are available as an opt-in experiment; normal builds keep buffered storage. Performance comparisons now require matching build/profile provenance and assess CPU and memory together.
