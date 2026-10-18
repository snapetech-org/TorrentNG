# Transfer Core Ticket Specification Template

Copy this section into the work log when a ticket starts. Keep code state,
repository test state, and external qualification state separate.

## `<TICKET-ID>` — `<short title>`

- **Priority / wave:**
- **Prerequisites:**
- **Owning component:**
- **Finding or capability:**
- **Status:** `todo` / `wip` / `done` / `blocked`

### Scope

Describe the behavior this ticket changes and the behavior it leaves for a
later ticket. Name the production route that should use the result.

### Ownership and invariant

- **Current owner:**
- **Target owner:**
- **Invariant preserved:**
- **Must not:**

### Acceptance

List observable outcomes, bounds, and failure behavior. For protocol and
storage work, include the valid, invalid, cancellation, late-completion, and
restart cases that apply.

### Files and compatibility

- **Expected code paths:**
- **State or wire compatibility:**
- **Migration / rollback:**
- **Feature flag or rollout boundary:**

### Evidence

- **Regression source added or updated:**
- **Command or probe to run:**
- **Expected result:**
- **Executed on exact revision:** `yes` / `no`
- **Artifact or report path:**
- **External evidence still required:**

Do not mark `done` while an acceptance item is unwritten or required evidence
is missing. If the code is complete but bulk tests are deferred, keep the
ticket `wip` and record that the tests are authored but unrun.
