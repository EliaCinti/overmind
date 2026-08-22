# ADR-0033: Invites and membership — several people, one company

- **Date:** 2026-08-22
- **Status:** accepted
- **Builds on:** [ADR-0032](0032-authentication-the-boundary-moves-off-the-machine.md), which sketched this milestone so M24 would build toward it.

## Context

M24 left registration open to whoever reaches the port, said so in the
threat model, and dated the fix: "deliberate until M25's roles-and-invites".
The owner's own ask is concrete: him on his Mac, a friend on their PC, and
eventually both inside one company — with the data safe.

Two mechanics are missing: a way to let the *right* people in (and only
them), and a way to say *which companies* each person is part of.

## Decisions

1. **Sign-up becomes invite-gated the moment anyone exists.** The first
   account ever created still signs up freely and owns the instance — a
   fresh install must be able to claim itself. From the second person on,
   sign-up requires an **invite code**: minted by the owner, single-use,
   seven days of validity, stored **hashed** like a session token (a leaked
   database mints no entries). The sign-up screen simply grows one field,
   shown only when the instance already has users: the flow the owner asked
   for — landing, sign-up, back — does not change shape.

2. **Membership is the filter, and it is organizational.** A
   `company_members` table says who is inside which company; creating a
   company makes you a member of it; the company list shows only yours, and
   the company-scoped surface (`/companies/{id}/…`) refuses non-members.
   The instance owner passes everywhere — the owner is the administrator,
   and pretending otherwise would be theater on a machine they control.
   - **Said plainly, as the threat model already says of brains:**
     membership is an organizational boundary, not a security boundary.
     Sub-resources addressed by bare id (a task, a session, an agent) are
     gated by authentication, not yet by membership; walking them to
     another member's company takes a valid account on an instance whose
     every account the owner invited. Tightening bare-id routes is this
     milestone's slice B, not an afterthought left unwritten.

3. **Members join a company by being added, not by invitation ceremony.**
   Any member of a company can add another registered user to it (a team
   invites a colleague; the owner is not a bottleneck). Removing members —
   and per-company roles beyond presence — wait until someone actually
   needs them; a permission system nobody asked for is complexity with no
   customer.

4. **The actor already in the audit chain becomes legible.** Events carry
   the acting user id since M24; the interface now shows *who* beside
   *what* where decisions surface (approvals, budget changes). No schema
   change: the id was in the hashed payload all along.

## Alternatives considered

- **Email invitations** — there is no mail infrastructure in a self-hosted
  box and no reason to build one: the owner hands the code over the channel
  they already share with the person they are inviting.
- **Per-company roles now** (admin/member/viewer) — nothing in the product
  differentiates them yet; deferred until a real need names the roles.
- **Open registration forever** — rejected; it was honest only as a dated
  exception, and the date is this milestone.

## Consequences

- `invites` and `company_members` tables; sign-up takes an optional
  `invite` that becomes mandatory once users exist; `/companies` filters by
  membership; company-scoped routes check it; an owner-only surface mints
  invites; a member surface adds a user to a company.
- The threat model's "sign-up is open" row is rewritten to describe the
  invite gate, and the "organizational, not security" honesty moves from a
  promise into the boundary table with its tests.

## Addendum — slice B delivered (22 Aug 2026)

The bare-id surface now answers the same membership question. The wall
resolves an id to its company through the row that owns it — one
prefix→query table: `/agents/`, `/approvals/`, `/artifacts/` (via its task),
`/meetings/`, `/notifications/`, `/org-proposals/`, `/projects/`,
`/sessions/` (via its task), `/tasks/`, `/tokens/` — plus the audit feed
filtered by `company_id`, which is the same surface read sideways. A
non-member gets the wordless 403 of decision 2; the owner passes as before.
An id that resolves to nothing is let through to the handler's 404: the
boundary is organizational, and a member asking about a vanished task
deserves the truth rather than a refusal that would only make sense against
an adversary the threat model does not claim to stop. Held by
`the_door.rs — the_bare_id_surface_is_gated_by_membership_too`.
