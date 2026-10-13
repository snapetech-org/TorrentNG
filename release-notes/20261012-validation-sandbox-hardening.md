---
category: security
audience: operators
area: ci-security
action: none
breaking: false
---

Local validation snapshots exclude dotted and extensionless credentials, private/API keys, access/refresh-token and client-secret material, suffixed dotenv credentials, and non-source files under credential folders. Source modules such as TokenStore.ts and safe templates such as tokens.env.example remain available; output logs have a visible truncation marker.
