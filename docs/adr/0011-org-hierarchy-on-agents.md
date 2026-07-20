# ADR-0011: Org hierarchy lives on agents (Paperclip model); drop the roles table

- **Date:** 2026-07-20
- **Status:** accepted (supersedes the M1 `roles` table)

## Context

M1 speculatively created a separate `roles` table (id, company_id, title, reports_to→roles) with `agents.role_id`, but gave it no API — it was always empty. M5 needs the org chart. Studying Paperclip's actual schema (per the alignment rule): Paperclip has **no roles table** — the hierarchy lives on `agents` themselves: `agents.reports_to → agents.id`, plus `agents.role` (text) and `agents.title`. Our own ARCHITECTURE.md already describes this ("an agent has one manager; the root is the human owner").

## Decision

Adopt Paperclip's model. Migration 0004:
- `agents.reports_to → agents.id` (the manager; `NULL` = reports to the human owner, the chart's root)
- `agents.title` (free-text job title)
- drop `agents.role_id` and `DROP TABLE roles`

Our `archetype` already answers "what kind of agent" (Overmind's addition), so we don't need Paperclip's `role` text column too; `archetype` + `title` cover it.

New surface: hiring accepts optional `title` + `reports_to` (manager must be an agent in the same company); `POST /agents/{id}/reassign` changes an agent's manager/title. **The reporting DAG is enforced server-side**: no self-reporting, and a reassignment that would close a cycle (walk up from the proposed manager; if we reach the agent, reject) returns 400. `reports_to` uses a distinguishing deserializer so `null` (move to top) differs from omitted (unchanged).

## Alternatives considered

- **Keep the separate `roles` table** (roles with their own reporting lines, agents assigned to roles) — a second hierarchy to keep consistent with the agent set, diverges from Paperclip, and nothing needed the indirection. Rejected; dropped the empty table.
- **Also add Paperclip's `role` text column** — redundant with our archetype. Rejected.
- **Enforce the DAG only in the UI** — a prompt-injected or scripted client could create cycles that break the org walk. Rejected: invariant enforced in the handler.

## Consequences

- `list_agents` now returns `title` + `reports_to`; the UI renders the reporting tree and edits it inline.
- Dropping a column + table in one migration relies on SQLite ≥ 3.35 `ALTER TABLE DROP COLUMN` (fine with bundled sqlx SQLite); verified by the migration running clean in every test.
- Projects → goals → tasks already cascade (M1/M2); M5 makes the *people* structure first-class. Auto-decomposition (a manager agent breaking a project into tasks) is agent behavior for a later milestone, not structural plumbing.
