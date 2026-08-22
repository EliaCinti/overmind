# ADR-0035: The estimate learns from the ledger

- **Date:** 2026-08-22
- **Status:** accepted
- **Builds on:** [ADR-0012](0012-budgets-and-governance.md) (the reservation at checkout), [ADR-0022](0022-conversational-spend-under-budget.md) (one arithmetic for tasks and turns), [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (the adapter's own ceiling).

## Context

Since M6 every checkout reserves a **flat** estimate — fifty cents, one
number for every agent, every model, every kind of work — and the gate asks
whether *spent + reserved + estimate* fits under the cap. M18 named the gap
("a turn is still not priced before it runs — the estimate is flat") and M20
narrowed it with `--max-budget-usd`, then measured that brake at 2.6×
overshoot on a five-cent ceiling. Two milestones carried the same sentence.

The flat number is wrong in both directions. A Sonnet writer whose turns
cost three cents is refused at a ten-cent cap, because fifty does not fit —
the cap *means* something and the agent never gets to spend it. An Opus
reviewer whose runs cost a dollar is waved through at a sixty-cent cap,
because fifty fits — and the ledger records the overrun after the fact. In
both cases the number Overmind reserved had nothing to do with the agent it
reserved it for.

Meanwhile the ledger has held the truth since M18: one `cost_event` per
adapter invocation, for tasks and turns alike, per agent.

## Decisions

1. **The estimate is this agent's own recent cost, read from the ledger.**
   At checkout — task or turn — the reservation is the **75th percentile of
   the agent's last ten costs of the same kind**: task runs summed per
   session, conversational turns one per event. Above the median on
   purpose: a reservation exists to keep the next run *inside* the cap, so
   it should lean toward the agent's dearer days rather than its average.
   Floored at one cent — a zero reservation would make the gate a formality.

2. **Until the ledger knows enough, the flat number stands.** Fewer than
   three samples of that kind and the estimate is what it always was:
   `OVERMIND_START_ESTIMATE_CENTS`, fifty by default. A guess from one data
   point is not better than a guess from none; it is the same guess with a
   false precision attached. The fallback is visible in the summary
   (`samples: 0`), so nobody mistakes it for a measurement.

3. **Task and turn are learned apart.** A task run carries its repository
   and its worktree into the context; a chat turn carries a conversation.
   They cost differently for the same agent, and the ledger already tells
   them apart by `session_id`.

4. **The ceiling is unchanged.** `--max-budget-usd` still rides on every
   run as the coarse brake M20 measured. This decision is about what the
   gate *reserves*, not what the adapter *enforces* — the two layers stay
   the two layers.

5. **What is reserved is said.** The budget summary carries, per agent, the
   estimate for its next task and its next turn and how many samples each
   rests on, so the life-line can say *"≈ 3¢ a turn, from 8 turns"* — and
   the person steering by the bar knows whether the number is the agent's
   or the default's.

## Alternatives rejected

- **A per-model price table** (tokens × rate) — rejected: Overmind does not
  know a run's token count before the run, the rate table would be a second
  source of truth beside the adapter's own `total_cost_usd`, and the fixed
  context cost that dominates small runs (M20 measured it) is exactly what
  a table cannot see and the ledger can.
- **A per-agent setting the person types** — rejected: it is the flat
  number again, moved onto the person; UX.md says a field the product can
  fill from what it already knows is a field it should fill.
- **The mean** — rejected in favour of the 75th percentile: a reservation
  that is right half the time lets the cap be crossed half the time.
- **A larger window** (all history) — rejected: an agent whose work changed
  (new model, new repository) would be priced on the past for months; ten
  is enough to smooth one odd run and short enough to follow a change.

## Consequences

- `governance::estimate_cents(conn, agent_id, kind, default) -> Estimate {
  cents, samples }`, used by the task checkout and by `ceo::run_adapter`;
  `check` and `reserve_turn` are untouched — they receive a number, as they
  always did.
- `GET /companies/{id}/budget` gains `estimates: { task: {cents, samples},
  turn: {cents, samples} }` per agent.
- M18's carried sentence is retired: a turn *is* priced before it runs, by
  the only instrument that has ever been right about it — the ledger.
