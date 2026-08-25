# ADR-0041: The CEO writes back on its own

- **Date:** 2026-08-25
- **Status:** accepted
- **Builds on:** [ADR-0022](0022-the-price-of-a-turn.md) (every turn is budget-gated), [ADR-0038](0038-from-the-ceos-plan-to-a-running-task.md) (`tasks.conversation_id` — the thread a task was born in), [ADR-0040](0040-a-conversation-outgrows-the-turn.md) (bounded threads).

## Context

The owner: *"quando necessario, dopo aver visto le risposte degli agenti,
Rune mi scriva da solo in autonomia per aggiornarmi."* Until now every chat
message was an answer: the CEO spoke only when spoken to, so a person who
delegated ten tasks had to poll the board to learn they had landed.

An unprompted agent voice is powerful and easy to make obnoxious. The design
question is not *can* it write, but *when it must not*.

## Decisions

1. **The trigger is material, not temporal.** A digest is due for a
   conversation only when a task **born in that thread**
   (`tasks.conversation_id`) has a **completed** session that finished
   *after the person's last word* in the thread. No timer talks; finished
   work talks.

2. **Debounced, and never on top of a turn.** A thread must have been quiet
   for `OVERMIND_DIGEST_DEBOUNCE_SECS` (default 180) — an update must not
   land between a person's own messages — and never while a turn is in
   flight there. `OVERMIND_CEO_DIGEST=off` disables the whole thing.

3. **The agent may deliberately stay silent.** The digest prompt shows the
   finished tasks (title + the agent's own report, clamped) and says: write
   a short update — what landed, what needs a decision — *or reply exactly
   `SKIP` if none of this is worth interrupting the user for*. Silence is a
   first-class answer.

4. **Never twice for the same completions.** An in-memory watermark per
   conversation (the newest `finished_at` announced *or skipped*) advances
   **before** the turn runs; a restart forgets it, and the worst case is one
   redundant ask that SKIP settles.

5. **An unprompted update opens no work.** Whatever the model returns, the
   plan's `tasks` are discarded in digest mode — enforced, not asked. An
   update the person did not request must not spend beyond its own turn,
   which remains budget-gated like every turn (ADR-0022).

## Consequences

- Delegation stops requiring polling: finish the espresso, Rune has written
  *"Tobia ha consegnato; serve una tua decisione su X"*.
- Cost is bounded: at most one turn per batch of completions per thread, and
  the agent's monthly cap brakes it like everything else.
- `tests/digests.rs`: one update per batch (a second beat repeats nothing);
  SKIP posts nothing and is not asked twice.
- Not done, deliberately: digests for meetings (the decision already returns
  to work by its own path, ADR-0020) and cross-thread digests — a thread's
  agent speaks only for its own thread.
