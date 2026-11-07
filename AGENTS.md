## Runner policy — current owner directive, 2026-11-06

All build, test, scan, package, and release jobs require compatible private
ephemeral JIT runners. Preserve the required native operating system and
architecture. There are no GitHub-hosted fallbacks. Missing private capacity
is a provisioning blocker; jobs remain queued or blocked until it is fixed.
This supersedes the earlier hosted-first instructions.

Keep public-fork code in disposable VM-backed JIT environments without secrets,
with minimal read access and restricted egress. Never widen trusted runner
access to admit untrusted code. Preserve active builds and other agents' work.
Do not restart the broker while it owns guests. Validate new workflow revisions
on private JIT; rerunning an old run uses its original workflow revision.

The shared policy and inventory live in
`snapetech-org/packer/scripts/shared-jit-runner-pool/`. Completion requires
source routing, deployed broker policy, runner-group access, and live capacity
to align; workflow YAML alone is insufficient.

CodeQL remains disabled by explicit owner directive. Do not restore or reroute
CodeQL unless the owner explicitly reverses that instruction.

## Git delivery

Standing authorization applies to task commits and pushes. Commit focused
milestones and push promptly to the remote default main/master branch, following
applicable review rules. Stage only task-owned changes, preserve unrelated work,
omit secrets, and never rewrite published history or make empty commits.
