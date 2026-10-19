# Keep stable container aliases in sync

Stable TorrentNG releases require publication to both GHCR and Docker Hub.
The native, sidecar, and phase1 images receive matching `:main` and `:latest`
aliases, and the Unraid templates follow `:latest`.
