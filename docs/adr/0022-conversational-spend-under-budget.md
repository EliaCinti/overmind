# ADR-0022: Conversational spend enters the ledger, and a room that runs out of money waits

- **Date:** 2026-08-06
- **Status:** accepted

## Context

[ADR-0012](0012-budgets-and-governance.md) (M6) made the monthly budget a hard
stop: at task checkout, inside the same transaction, `spent + reserved +
estimate > cap` refuses the run, records a `budget_incidents` row and an audit
event, and leaves the task untouched. That mechanism has been correct since it
shipped, and the ROADMAP has carried a known gap beside it:

> Chat and meeting turns do not reserve budget. […] A meeting can spend up to 13
> adapter runs without the agent's monthly cap being consulted. **This is the
> most pressing hole.**

Reading the code, the hole is one layer deeper than that sentence. Conversational
turns do not merely skip the *reservation* — **they record no cost at all**.
`cost_events` is written in exactly one place, `runner.rs`, at the end of a task
session. Chat turns (M12) and meeting turns (M13) go through `ceo::run_adapter`,
which returns the adapter's stdout to a caller that reaches for the agent's plan
and drops everything else. The cost is not unavailable — `ceo::plan_json`
deliberately *steps over* the adapter's own result envelope, the very object
carrying `total_cost_usd`, to find the plan behind it. It is read and discarded.

The consequence is not confined to conversation. `governance::spent_cents` sums
`cost_events`, so every agent's recorded spend is missing all of its
conversational work. The M6 gate is therefore arithmetically correct on a ledger
that is incomplete: a task checkout is refused, or allowed, on a number that
does not include the meetings the same agent sat in. The hard stop is real; what
it is measuring is not.

This is the third field in the same family as `permissions` (M14 slice 1) and
`model` ([ADR-0021](0021-function-domain-characterization.md)): a mechanism that
looks enforced, is believed, and runs on data that never arrives.

## Decision

### One ledger

Every adapter invocation records a `cost_event`, whatever caused it. Task runs
already do. Chat turns and meeting turns now do too, with `task_id` and
`session_id` null — both columns have been nullable since M2, so the ledger's
shape already anticipated this. `spent_cents` needs no change; it stops
under-reporting.

### One cap, checked per turn

The budget check moves into `ceo::run_adapter`, the single choke point every
non-task invocation passes through. Before spending, in one transaction:
`spent + reserved + turn estimate > cap` refuses the turn and records the same
`budget_incidents` row and audit event the task path records.

Reservations for turns cannot live on `agent_task_sessions` — that table's
`task_id` is `NOT NULL`, and bending it so a chat turn could impersonate a task
session would corrupt the one thing that table means. A dedicated
`agent_turn_reservations` table holds them, and `governance::reserved_cents`
becomes the sum of both sources. A turn reserves before it runs and releases
when it ends, however it ends.

The check is **per turn**, not per meeting: a room that can afford four turns
runs four turns. Pre-reserving a whole meeting would refuse rooms that would
have fitted, and would have to guess a number nobody can know before the first
agent speaks.

### A room that runs out of money is paused, not closed

Running out of budget is **transient and external** to the deliberation: it says
nothing about the topic, the participants, or how well the room was doing. So it
must not destroy the room's work, and it must not manufacture a conclusion from
it either.

The meeting takes a new status, **`paused`**. No turn is spent, the transcript
so far is already durable (`meeting_turns` has persisted every turn with its
`ordinal` since M13), and you are notified: which agent ran out, what it has
spent against what cap, and that raising the cap — or waiting for the window to
roll over — is what resumes it. Meanwhile nothing else is blocked: agents with
budget keep working, other rooms keep running.

Resuming re-enters the same loop at `ordinal = turns recorded so far`. This is
exact rather than approximate because the speaker is `speakers[ordinal %
speakers.len()]` — a pure function of the ordinal — so the round-robin lands on
the same agent it would have. **The turn cap does not refill.** A paused and
resumed room gets the turns it was approved for, never more; otherwise pausing
becomes the way to buy extra deliberation the human never allowed.

Resuming is **manual**, an action you take. Automatic resumption on a new
billing window would silently restart rooms nobody is watching, which is the
opposite of M13's whole doctrine that nothing runs until you say so.

A **paused room counts against M13.5's per-company limit** on rooms waiting on
the human. That limit exists to stop rooms piling up unnoticed, and a paused
room is exactly a room piling up unnoticed; leaving it uncounted would reopen
the hole M13.5 closed from a new direction.

### In chat, the refusal is an answer

A chat turn has no multi-turn state to preserve, so there is nothing to pause. A
refused turn replies in the thread rather than failing the request: what has been
spent, what the cap is, and that raising it or waiting unblocks it. A person who
asked a question deserves an answer, and "your agent is out of money" is one.

## Alternatives considered

- **The chair closes the room with what it has.** Reuses M13's turn-cap
  machinery, so a room that runs out still produces a decision. Rejected on
  reflection: it forces a call under artificial pressure and then presents it as
  an ordinary decided meeting — `decisions_block` injects it into every
  participant's next run identically to one that deliberated in full, so the
  "closed early" caveat exists in the record and reaches nobody who acts on it.
  It also destroys recoverable work over a condition that a top-up fixes.
- **Warn but never block conversational turns.** Half the fix, and the honest
  half — accounting becomes true and tasks stop computing on a partial ledger.
  Rejected because the cap would stay something meetings can exceed, which is
  the sentence the ROADMAP calls the most pressing hole. A cap with a documented
  way around it is a suggestion.
- **Refuse the turn and fail the meeting**, exactly like a task checkout. Least
  code, most consistent with M6 — and it leaves rooms half-deliberated with
  nothing to show, which M13.5 already identified as the outcome to avoid.
- **Reserve the whole meeting up front** (`turn_cap × estimate`). Simple to
  reason about, wrong in both directions: it refuses rooms that would have
  fitted, and reserves against a cap for turns that never run because someone
  decides on turn two.
- **Exempt conversation with the org leader**, so talking to your own CEO is
  never blocked. Tempting for the UX, rejected as a hole of exactly the kind
  this ADR closes: an exemption is how a cap stops being a cap.

## Consequences

- **Migration 0018** adds `agent_turn_reservations` and the `paused` status.
  Nothing is backfilled: conversational spend before this ADR was never recorded
  and cannot be reconstructed, so the ledger becomes correct going forward and
  does not pretend about the past.
- **Recorded spend will jump** the first month after this ships, on agents that
  chat or meet. That is the measurement arriving, not the spend rising — worth
  saying out loud before someone reads it as a regression.
- **A new meeting status reaches the UI by construction.** M16 keys the status
  table on the server's own values, so the web build fails until every language
  names `paused` — the mechanism working as designed.
- **`run_adapter` gains the company and agent it is running for.** It could
  previously be called with only traits, which is precisely why it was possible
  to spend without anyone's budget being consulted.
- **`parse_cost` moves from private to crate-visible** in `runner.rs`, since the
  ledger now has two writers. It stays one implementation.
- **Deliberation becomes resumable**, which means it must become restartable
  from a stored offset rather than a single straight-line call. That is the real
  cost of this decision, and it is paid once.
- **What this does not do:**
  - It does not price a turn before running it. The estimate is a flat
    reservation, as it is for tasks; a real pre-flight price would need token
    counting we do not do. An agent can still overrun its cap by the difference
    between the estimate and one turn's true cost.
  - It does not detect **subscription exhaustion** — the adapter failing because
    the underlying Claude subscription is out of tokens. That is a different
    failure from our cap: ours is known before spending, that one arrives as an
    adapter error we would have to recognise heuristically from stdout. The
    pause path is where it belongs when we can identify it reliably; until then
    it surfaces as an adapter failure like any other.
