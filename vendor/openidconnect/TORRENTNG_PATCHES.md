# TorrentNG OpenID Connect patches

This directory vendors the published `openidconnect` 4.1.0 crate from
<https://crates.io/crates/openidconnect/4.1.0> (upstream repository:
<https://github.com/ramosbugs/openidconnect-rs>). The upstream MIT license is
retained in `LICENSE`.

TorrentNG adapts the crate's RSA verification and signing implementation to
`rsa` 0.10.0 and `sha2` 0.11.0 because the published crate still selects the
vulnerable `rsa` 0.9 series. This closes RustSec RUSTSEC-2023-0071
(CVE-2023-49092). The `crypto-bigint` issue RUSTSEC-2026-0387 is addressed by
using the upstream 4.1.0 release, whose dependency resolution selects
`crypto-bigint` 0.7.11.

The RSA JWK leading-zero verification regression test and deterministic PKCS#1
v1.5 signature vectors are retained. RSA-PSS tests verify signatures by
round-trip because their signatures are randomized.
