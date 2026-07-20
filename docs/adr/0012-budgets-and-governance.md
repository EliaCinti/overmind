# ADR-0012: Budgets, approval gate, agent lifecycle, config revisions

- **Date:** 2026-07-20
- **Status:** accepted

## Context

M6 is the governance layer — what makes Overmind safe to leave running. Paperclip has `budget_policies`/`budget_incidents`, `approvals`, and `agent_config_revisions`. The acceptance criteria: an over-budget start is stopped server-side, and a gated action blocks until human approval.

## Decisions

### Budgets (hard stop, atomic with checkout)

- The budget **amount is the agent's `monthly_budget_cents` trait**, not a separate `budget_policies` row (recorded deviation — one source, no extra config surface). The window is the calendar (UTC) month.
- At task start, **inside the checkout transaction**: `spent(this month) + reserved(in-flight) + estimate` must fit under the cap, or the start is refused (HTTP 402) and a `budget_incidents` row is written. Since it shares the transaction with the conditional checkout UPDATE, concurrent starts can't collectively overrun the cap.
- A per-session **reservation** (`agent_task_sessions.reserved_cents`, = `OVERMIND_START_ESTIMATE_CENTS`, default 50) covers the gap between checkout and the real `cost_event`; it's released to 0 on finalize. `budget=0` means uncapped.

### Approval gate (blocks until approval)

- A per-agent `requires_approval` flag. When set, `start_task` files a pending `approvals` row (`type=task_start`, payload = task+agent) and **launches nothing**; the task stays `todo`. `POST /approvals/{id}/decision` approves (→ runs the start with the gate bypassed) or rejects (→ leaves it). This is `start_task(…, bypass_approval)`; the wakeup scheduler goes through the same gate.

### Agent lifecycle

- `pause` / `resume` / `terminate` (status `active`/`paused`/`terminated`). Paused/terminated agents can't be started (`RunnerError::Blocked` → 409); terminate is permanent.

### Config revisions + rollback

- Every hire and reassignment appends an `agent_config_revisions` row (before/after JSON snapshot, changed keys). `POST /agents/{id}/rollback` restores a past revision's config and **appends a new `rollback` revision** — history is forward-only, never rewritten. Mirrors the append-only spirit of the audit log.

## Alternatives considered

- **A separate `budget_policies` table** (per Paperclip) — more faithful, but the agent already carries its budget; a second table to keep in sync earns nothing yet. Deferred; may return if budgets need to scope to teams/company.
- **Reserve the real cost, not a flat estimate** — impossible before the run; the flat reservation is a safe upper-ish bound that's reconciled immediately on finish.
- **Autonomy `act_with_approval` as the gate** — conflates "who starts work" (M3) with "this needs sign-off"; a distinct `requires_approval` flag is clearer and composes with any autonomy.
- **Editing the target revision on rollback** — would lose history; appending is auditable and reversible.

## Consequences

- `RunnerError` gains `OverBudget` (→ 402) and `Blocked` (→ 409); `start_task` now returns `StartResult::{Started, ApprovalRequired}`, so all callers (API, scheduler, approval decision) branch on it.
- The UI gains an approvals inbox (bell + count), per-agent budget bars in the org chart, and pause/terminate/approval-gate controls. Config-revision browsing/rollback is API-complete and tested; a revisions UI is deferred.
- Warn-threshold incidents (80%) and notifications are modeled in the incident table shape but not yet emitted — hard-stop is the shipped enforcement.
