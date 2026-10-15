---
category: changed
audience: operators
area: ci
action: none
breaking: false
---

Trusted main validation can use the shared runner pool when its readiness flag is enabled; pull requests and unready runs continue on GitHub-hosted runners. WebUI browser checks choose a run-scoped preview port so concurrent workflows on the same runner do not collide.
