# Dirty performance worktree snapshots — 2026-10-23

These are exact snapshots of the nine remaining dirty performance worktrees found during the requested final integration audit. Every worktree's `HEAD` is already an ancestor of `main`; the changes below were uncommitted at capture time.

They are preserved as patch artifacts rather than applied to the production source. The associated measured-screen reports mark these candidates as not promoted, inconclusive, or losing. Keeping the exact dirty deltas available lets Astra reassess them without changing the current best configuration. The worktree snapshots themselves were not separately benchmarked or retested after their recorded screens.

Each patch is `git diff --binary HEAD` from the recorded base commit; it can include staged and unstaged tracked changes. The lazy-worker worktree also contained one untracked release executable, archived here as `torrentngd-lazy-worker.elf.gz`; its uncompressed SHA-256 is in `manifest.json`. All patch and binary hashes are recorded in the manifest.

| Worktree | Base | Existing screen record | Captured delta | Status entries |
|---|---|---|---|---:|
| `active-stream-current-main-worktree` | `4b187a8deda8` | [`active-stream-current-main-screen-20261023`](../active-stream-current-main-screen-20261023/README.md) | [`active-stream-current-main-worktree.patch`](active-stream-current-main-worktree.patch) | 1 |
| `current-thread-runtime-worktree` | `a48a9c0b0f79` | [`current-thread-runtime-screen-20261022`](../current-thread-runtime-screen-20261022/README.md) | [`current-thread-runtime-worktree.patch`](current-thread-runtime-worktree.patch) | 2 |
| `lazy-second-worker-worktree` | `dc298249eb74` | [`lazy-worker-trigger-screen-20261023`](../lazy-worker-trigger-screen-20261023/README.md) | [`lazy-second-worker-worktree.patch`](lazy-second-worker-worktree.patch) | 2 |
| `peer-codec-admission-fastpath-worktree` | `6683d993a979` | [`peer-codec-capacity-fastpath-screen-20261023`](../peer-codec-capacity-fastpath-screen-20261023/README.md) | [`peer-codec-admission-fastpath-worktree.patch`](peer-codec-admission-fastpath-worktree.patch) | 1 |
| `peer-read-current-stream-worktree` | `4d13f9d0ecdc` | [`peer-read-current-entry-screen-20261023`](../peer-read-current-entry-screen-20261023/README.md) | [`peer-read-current-stream-worktree.patch`](peer-read-current-stream-worktree.patch) | 1 |
| `peer-request-decode-screen-20261023-worktree` | `c10e914db6ee` | [`peer-request-decode-fastpath-screen-20261023`](../peer-request-decode-fastpath-screen-20261023/README.md) | [`peer-request-decode-screen-20261023-worktree.patch`](peer-request-decode-screen-20261023-worktree.patch) | 2 |
| `select-biased-worktree` | `672e5233fb77` | [`peer-select-biased-screen-20261023`](../peer-select-biased-screen-20261023/README.md) | [`select-biased-worktree.patch`](select-biased-worktree.patch) | 1 |
| `upload-batch-16-screen-20261023-worktree` | `395b1dd9ab03` | [`upload-batch-16-screen-20261023`](../upload-batch-16-screen-20261023/README.md) | [`upload-batch-16-screen-20261023-worktree.patch`](upload-batch-16-screen-20261023-worktree.patch) | 2 |
| `upload-mutex-batch-screen-20261023-worktree` | `acf6610eed93` | [`upload-obligation-lock-batch-screen-20261023`](../upload-obligation-lock-batch-screen-20261023/README.md) | [`upload-mutex-batch-screen-20261023-worktree.patch`](upload-mutex-batch-screen-20261023-worktree.patch) | 3 |

## Applying a snapshot

Check a patch against its recorded base before applying it. These are review artifacts and may need adaptation to current `main`:

```sh
git checkout <base-commit>
git apply --check <snapshot.patch>
```

The neighboring screen reports contain the prior measurements, correctness notes, and promotion decisions. A patch archive records code; it is not evidence of a performance win.
