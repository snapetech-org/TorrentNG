---
category: changed
audience: operators
area: performance
action: none
breaking: false
---
Experimental PGO builds can use a profile-informed function order after validating the source, compiler, and profile. Benchmark manifests reject changed source trees or shared libraries, so comparisons stay tied to the binary actually measured. Queued pull request release-note checks keep the original base commit after merges.
