# OIDC authentication

TorrentNG supports server-side OpenID Connect authorization-code sign-in with
PKCE for ordinary providers. Apple uses its documented form-post flow without
PKCE. A successful callback creates the WebUI's normal signed, HttpOnly
`tng_session` and qBittorrent-compatible `SID` cookies. Provider tokens are
not returned to the browser or persisted. Password and API-token login remain
available. This adapter follows the [shared cross-project OIDC contract](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-cross-project-template.md).

Configure a confidential web client with this exact callback path:

```toml
[auth.oidc]
provider = "keycloak"
issuer_url = "https://auth.example.lan/application/o/torrentng/"
client_id = "torrentng"
client_secret = "replace-me"
redirect_uri = "https://torrent.example.lan/api/v1/auth/oidc/callback"
allowed_emails = ["operator@example.com"]
allowed_email_domains = []
allow_any_verified_user = false
allow_private_issuer = false
```

The equivalent environment variables use the `TNG_OIDC_` prefix (the legacy
`RTNG_OIDC_` prefix is also accepted): `ISSUER_URL`, `CLIENT_ID`,
`CLIENT_SECRET`, `REDIRECT_URI`, `ALLOWED_EMAILS`, `ALLOWED_EMAIL_DOMAINS`,
`ALLOW_ANY_VERIFIED_USER`, and `ALLOW_PRIVATE_ISSUER`. Email and domain lists
accept comma, semicolon, or whitespace separators. All four core settings must
be supplied together. Issuer metadata must exactly match the configured
issuer. The callback must use HTTPS and its path must be
`/api/v1/auth/oidc/callback`.

Set `TNG_OIDC_PROVIDER=keycloak` (or `RTNG_OIDC_PROVIDER=keycloak`) to brand
the login button and choose its bundled provider mark. This optional hint does
not change the issuer or validation. See the [provider preset catalog](https://github.com/snapetech-org/seerrng/blob/main/docs/maintainers/oidc-provider-catalog.json)
for supported IDs and official setup links.

Admission requires a verified email matching an exact listed address or
domain. An empty allowlist denies sign-in unless
`allow_any_verified_user = true` is explicitly enabled. TorrentNG's current
WebUI has one operator permission level; every admitted identity receives that
same full operator access. Provider group claims do not grant or change
privileges, and an OIDC identity is not linked to an existing local login by
email. Local password and API-token login remain recovery paths.

Issuer and discovered endpoint hosts that resolve to private or reserved
addresses are rejected by default; redirects from provider endpoints are
disabled, and each request is pinned to its validated addresses to prevent DNS
rebinding. OIDC requests connect directly and bypass environment proxy settings.
Set `allow_private_issuer = true` only when the configured provider and its
endpoints are intentionally hosted on a private network. The callback still
must use HTTPS.

The flow stores a one-time state and nonce (plus a PKCE verifier for ordinary
providers) on the server for ten minutes, binds state to a Secure, HttpOnly,
SameSite=Lax callback cookie for ordinary providers,
validates the provider signature and required ID-token claims, and restricts
the post-login return path to a local path. Pending state is process-local, so
a service restart invalidates in-flight sign-ins. OIDC sessions use the
existing 24-hour app-session lifetime.

### Sign in with Apple

Set `issuer_url` to exactly `https://appleid.apple.com`, use the Apple Services
ID as `client_id`, and set `client_secret` to Apple's signed client-secret JWT.
Register `https://<public-host>/api/v1/auth/oidc/callback` as the exact HTTPS
return URL. TorrentNG requests only `openid email`, uses
`response_mode=form_post`, and omits PKCE for Apple's documented web flow. The
callback accepts Apple's bounded form POST and binds it to the one-time state
cookie. Apple may encode `email_verified` as a JSON boolean or string; only
`true` is admitted. Rotate the JWT before its six-month maximum expiry. See
Apple's [environment setup](https://developer.apple.com/documentation/signinwithapple/configuring-your-environment-for-sign-in-with-apple)
and [web authorization flow](https://developer.apple.com/documentation/signinwithapple/incorporating-sign-in-with-apple-into-other-platforms).
