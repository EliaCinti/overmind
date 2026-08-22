# ADR-0034: Deleting a company — a hard delete with an audit trail that stays

- **Date:** 2026-08-22
- **Status:** accepted
- **Builds on:** [ADR-0033](0033-invites-and-membership.md) (membership as the filter) and the audit chain's append-only stance (M1, M24).

## Context

The owner's first real test of the product found the missing verb: there is
no way to remove a company. His test companies could only be cleaned up by
surgery on the volume — deleting the data directory, and with it everything
else. The ROADMAP carries it as the first open item of M25.

Two precedents in this codebase argue *against* deleting rows:
`revoke_company_token` keeps a revoked credential as a timestamped row
"because the audit events naming one should still point at something", and
`terminate_agent` is a status write, not a `DELETE`. A company is named by
nearly every audit event — so which stance wins?

## Decisions

1. **A hard delete: rows, brain, and debris on disk.** The wound this heals
   is *disk that cannot be reclaimed* — a soft `deleted_at` flag would leave
   the owner exactly where he started, hiding the company while keeping its
   megabytes. The token precedent does not transfer: a token row is kept so
   the audit log has something to point at, but the audit log itself is the
   thing that outlives a company (see 3). Everything else goes: every table
   that names the company, children first inside one `BEGIN IMMEDIATE`
   transaction, then the brain directory, attachment and artifact
   directories, worktrees and meeting transcripts — best-effort, after the
   commit, because a directory that would not delete is disk to reclaim by
   hand, not a reason to claim the company still exists.
   - The foreign keys stay ON as a net: a table this handler forgot fails
     the whole transaction instead of leaving orphans.
   - The brain's cached MCP pool is dropped **before** its directory is
     removed — live server processes must not keep handles into a deleted
     tree.

2. **Deleting is a member's verb.** Membership is the filter here as
   everywhere on the company-scoped surface (ADR-0033): any member can
   delete the company, the instance owner passes as the administrator, and
   outsiders get the same wordless 403 as for any other room they are not
   in. There are no per-company roles to gate on — inventing an
   owner-of-the-company tier for this one verb would be theater, and
   ADR-0033 already deferred per-company roles until someone needs them.
   What guards against a slip is the interface, not a permission: the
   confirmation is typing the company's name, the one verb in the app that
   asks for a copy rather than a click.

3. **The audit chain is the one thing that stays.** `audit_events` carries
   no foreign key to `companies` and its append-only triggers abort any
   thinning, so the chain still verifies after the delete — every event for
   this id now points at nothing, and a new `company.deleted` event (with
   the name in its payload) is what says that is deliberate rather than
   corruption. History that a company existed, worked and was deleted is
   exactly what an audit log is for. The events remain reachable by id via
   `GET /api/audit/events?company_id=…`.

4. **A live session holds the door.** A queued or running session is an
   agent mid-thought; deleting the ground under it would leave the runner
   finalizing into missing rows. The request refuses with **409** until the
   work settles or is terminated. Nothing is force-killed: there is no
   process-kill mechanism today, and inventing one inside a delete handler
   would be two features in one commit.

## Consequences

- `DELETE /api/companies/{id}` — 200 `{ok: true}`, 404 for a company that
  never was or is already gone, 409 while sessions run, 403 for
  non-members (from the membership wall, for free).
- The UI's selection falls back to whatever company remains, or to
  onboarding when none does — deleting your last company lands you exactly
  where a fresh instance starts.
- Audit events for a deleted company are orphan pointers by design; the UI
  has no surface for them once the company is off the picker. Acceptable:
  the chain's integrity is what is promised, not a museum for every id.
- `code` worktree directories are swept, which can leave a user repo with
  stale `git worktree` metadata (`git worktree prune` cleans it) — same
  stance the runner already takes toward a vanished worktree.
