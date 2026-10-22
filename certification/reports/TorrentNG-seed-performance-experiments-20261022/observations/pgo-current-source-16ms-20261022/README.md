# Current-source PGO profile screens

These experiments use source commit `6cfc6a9aba79c954624d9d86f6508fb49ec0a894`, Rust 1.97.0, LLVM 22.1.6, a private loopback seed, 16 KiB blocks, a 32-block window, a 4 MiB/s receiver, and `pread`. The 64 MiB training fixture hash is `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`; the scored 128 MiB fixture hash is `7fae01c69f5254b9fbf3cdf6db292cc5da49ee3fa4bfbb995de8aa3b66dd6e67`.

## Transfer-window profile

LLVM counters were reset at the first scheduled upload batch. One verified 64 MiB run produced profile `80fda6a6a1da618cfc7e6bf364ea90c6e27d87cc7278bcc6c1fbfea151580b82`, used to build binary `ce7e769b49af69509f71c031efa22b2af1c1a0e31fc453e848f59ea809513039`.

Five 128 MiB pairs compared that binary with the existing 16 ms PGO binary, SHA-256 `8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`, whose archived profile predates the io_uring worker-selection fix. The old binary is preserved in [`../upload-coalesce-16ms/executables/`](../upload-coalesce-16ms/executables/).

| Build | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| Existing-profile control | 3.1588 | 16.5781 MiB | 6.4727 MiB |
| Fresh transfer-window profile | 2.9974 (-5.1%) | 18.4414 MiB (+11.2%) | 6.1719 MiB (-4.7%) |

All ten transfers verified and completed their 32-second payload window. Three pairs ran control first and two ran the candidate first. This is a repeated screen with an imperfectly balanced order, not a release qualification. The PSS regression prevents promoting the profile.

## Full-process profile

This profile counted from daemon startup rather than resetting at the first upload. A verified 64 MiB training run produced profile `47d37cf2ba582152f86fc8e3cd9d34998f0f8485fc4433082b10c47fd66de581` and binary `635a9cf74459ee3b63dd9279910c44ebe62a2baf68a7ba6118ef2e25fcae66cd`.

One verified 128 MiB pair measured 3.4244 vs. 3.0402 CPU-s/GiB (+12.6%), 16.7373 vs. 15.9141 MiB PSS (+5.2%), and 6.5273 vs. 6.0820 MiB anonymous memory (+7.3%) for the new full-process profile and existing-profile control, respectively. The full-process candidate ran first. This is screening evidence only and it did not improve any measured resource metric.

## Files

- `executables/` contains the exact instrumented training daemon and both PGO outputs, gzip-compressed. `SHA256SUMS.uncompressed` records the extracted binary hashes.
- `profiles/` contains the raw LLVM profile files and merged profiles, gzip-compressed. Its checksum file records uncompressed hashes.
- `*-training/` contains normalized and original probe summaries.
- `*-screen-*/` contains normalized and original summaries for each scored run.
- `runs/` contains fixture metainfo, configs, client logs, metrics, and compressed process `smaps`. Payload copies, API tokens, session databases, and bootstrap credentials are excluded.

Both profile-use builds completed without profile-mismatch warnings. The comparison did not include rTorrent. These profiles and binaries are retained as tradeoff evidence; the 16 ms release default and earlier PGO memory best remain unchanged. See the [experiment report](../../README.md) for the broader results.
