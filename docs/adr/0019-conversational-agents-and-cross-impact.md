# ADR-0019 — Conversational agents & cross-impact

- Status: accepted
- Date: 2026-07-28
- Supersedes/extends: [ADR-0018](0018-conversational-ceo-and-attachments.md) (conversational CEO), builds on [ADR-0005](0005-structured-agent-characterization.md) (structured characterization), [ADR-0006](0006-audit-log-and-task-lifecycle.md) (audit).

## Context

ADR-0018 gave the company one conversational surface: the **CEO** thread. The
user states intent, the CEO decomposes it into tasks and dispatches. But a
company is not only its CEO. The user wants to talk to a **specific agent in its
role** — "ask the Security Engineer directly", "show the Media & A/V specialist
this disc" — and, crucially, that conversation must be able to **ripple onto the
rest of the team**: what you tell one agent can create work for others.

Talking to a single agent in a silo would be a regression toward a generic
"chat with N bots". The value is a company that coordinates: address anyone, and
the org still moves as one.

## Decision

**1. Every agent is conversational, not only the CEO.** A conversation is with
an agent. The user can open a thread with any active agent.

- Schema: `conversations(id, company_id, agent_id, title, created_at)` with
  `UNIQUE(company_id, agent_id)` — one thread per agent per company (migration
  0009 renames `ceo_agent_id → agent_id` and swaps the company-only unique index
  for a per-agent one). The "CEO conversation" is simply the thread with the
  org leader (`reports_to IS NULL`); no special table.

**2. The agent turn is role-aware.** One code path, `run_agent_turn`, builds the
prompt from the agent's own characterization (archetype, traits, `custom_brief`,
title) plus the team, the history, memory (ADR-0013), and attachments (ADR-0018).
The **leader** is framed to dispatch broadly; a **specialist** is framed to do
its part and flag cross-impact. The agent returns a single validated JSON plan:

```json
{
  "reply": "<message to the user>",
  "tasks": [ { "title": "...", "description": "...", "execution_kind": "knowledge|code", "assignee": "<teammate name, optional>" } ],
  "escalate": "<optional note when the request affects the wider org>"
}
```

**3. Cross-impact is concrete and surfaced, never silent.**

- `tasks[].assignee` (optional): resolved by name/title to an **active teammate**;
  the task is created with that `assignee_agent_id`. This is the ripple — a
  conversation with agent A can open work for agent B. Unmatched/absent → the
  task is unassigned (team backlog).
- `escalate` (optional, specialists only): posted as a **system message in the
  leader's thread**, so the human sees the ripple raised to the CEO. The leader's
  own `escalate` is ignored (they are the top).

**4. Structured-first and fully audited** (ADR-0005/0006). The agent's actions
are validated JSON, never free prose that mutates state. Task creation,
assignment, and escalation each append audit events; the chain still verifies.

## Consequences

- API becomes agent-scoped: `GET|POST /companies/{id}/agents/{agent_id}/conversation…`.
  The old company-level `/conversation` (CEO-only) is replaced; the client talks
  to the leader by passing the leader's id, and to anyone else the same way.
- The Chat UI gains an **agent switcher** (talk to the CEO or any teammate); the
  composer and attachments are unchanged per thread.
- This is the substrate for **meetings (M13)**: bounded multi-agent deliberation
  is several agent turns with a recorded decision. Escalation here is the seed of
  "loop in the CEO".
- Not yet: cost/budget applied to conversational turns (still deferred), and the
  specialist actually *executing* from chat (it proposes/dispatches; execution
  stays on the board via the runner).

## Rejected alternatives

- **Only the CEO is conversational** (ADR-0018 as-is): rejected — the user
  explicitly wants to address any agent.
- **Free-form group chat**: rejected — deliberation is bounded and structured
  (that is M13), not an open channel.
- **Silent side-effects** (one agent quietly changes another's work): rejected —
  every ripple is a typed, audited task or a visible escalation message.

## Where this lives

- `crates/overmind-server/migrations/0009_agent_conversations.sql` — per-agent threads.
- `crates/overmind-server/src/ceo.rs` — `run_agent_turn` (role-aware), assignee
  resolution, escalation to the leader's thread. (Module name kept; it is now the
  general conversational layer.)
- `crates/overmind-server/src/api.rs` — agent-scoped conversation routes.
- `web/src/components/Chat.tsx` — agent switcher.
