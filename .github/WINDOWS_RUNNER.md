# Private Windows JIT route

The Windows Smoke workflow is dispatched from trusted main and targets the
private `packer-windows` runner group with the `packer` one-job runner. TorrentNG
release jobs `native-binaries` and `desktop-bundles` use the same Windows x64
route. No GitHub-hosted Windows fallback is configured.

The Packer group and Windows JIT capacity must be restored before these jobs
can run. Every Windows job has the exact common labels `self-hosted`,
`Windows`, `X64`, `on-demand`, `packer-windows`, and `TorrentNG`, plus one
run-scoped label derived from its workflow job ID. The broker verifies that
label against the workflow source marker and the queued job. Its policy must
include the `windows-smoke.yml` and `release.yml` main workflow refs, and it
must accept only qualified release tags whose commits are ancestors of `main`.
The release workflow checks tag ancestry before starting platform build jobs.

The private Windows image includes Visual Studio C++ Build Tools. The
`scripts/enable-msvc-environment.ps1` helper loads its x64 compiler variables
for the Windows MSVC release target.
