## Exact benchmark executables

These gzip files preserve the exact release and PGO binaries used in the coalescing comparisons. Decompress and check SHA-256 against SHA256SUMS.uncompressed; normalized result summaries record the same hashes.

The PGO pair used full-profile data SHA-256 45c1f0352f9fe9f1622b783ced1a48a93269cc2eb79ce77549468e35401fc4ed. The profile predates the io_uring worker-selection fix, so the candidate build emitted three known storage-backend control-flow mismatch warnings; both PGO arms use the same profile data.
