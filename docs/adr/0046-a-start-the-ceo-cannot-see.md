# ADR-0046: A start the CEO cannot see is a start it will lie about

- **Date:** 2026-09-02
- **Status:** proposed
- **Builds on:** [ADR-0038](0038-from-the-ceos-plan-to-a-running-task.md) (a planned task is offered to work the way its agent's autonomy says), [ADR-0012](0012-budgets-and-governance.md) (the budget gate), [ADR-0042](0042-the-ceo-runs-the-floor.md) (the CEO starts, relaunches and chains work on its own plan).

## Context

Measured on the owner's own *TravelAgency* company, 2 September 2026. Three
times in one conversation the CEO wrote *"adesso ne ho messe in run tre"* —
now I have put three in run — and the board stayed at **zero in progress**. By
the third time it had begun apologising for the wrong thing: *"ho descritto
delle partenze invece di eseguirle"*, I described starts instead of executing
them. That was a reasonable guess and it was false. It had executed them.

What actually happened, from the audit chain and the code:

- The CEO's reply and its `"start": [...]` list are **one JSON object**. The
  reply is delivered, and *then* the server tries the starts. The words are
  written before the outcome exists, so the CEO cannot have seen it.
- `runner::start_existing` already reports what it did — `Ok(None)` when no
  open task carries that title or the task has no assignee, and otherwise
  `Ok(Some(Offer))` where `Offer` is `Started`, `Asked { approval_id }` or
  `Waiting` ([ADR-0038](0038-from-the-ceos-plan-to-a-running-task.md)). **`ceo.rs` throws all
  of it away** and matches only on `Err`, which it writes to `eprintln!`.
- So one sentence — "it is running" — covers five different outcomes: it
  started; it is sitting in the inbox waiting for you; it is merely proposed
  and nobody was asked; no task by that title exists; the budget gate refused
  it.
- On this company it was the last two. `Chiara` had spent **€50.38 against a
  €50.00 cap**, so every start was refused: six `budget.blocked` events between
  11:21 and 13:13. And `governance.rs` writes that event to the chain and
  **notifies nobody**. The other two starts named tasks by a paraphrase rather
  than by the board's exact title — the titles are long sentences — so they
  matched nothing and returned `Ok(None)`, which is discarded without even a
  log line.

The chain knew. The server's stderr half-knew. The person was told the
opposite, three times, by the one part of the product whose whole job is to
tell them what is happening.

## Decision

1. **The CEO never reports a start whose outcome it has not been told.** The
   server appends the outcome to the reply it is about to deliver — one short
   line per start that did not simply run, composed through `i18n` in the
   company's language, the way every other sentence the server writes for a
   person is. "Started" adds nothing: the board already shows it.
2. **`start_existing` stops saying `Ok(None)` for two different things.** No
   task by that title and a task with nobody on it are different facts with
   different remedies, and the caller cannot act on a shrug. It returns an
   outcome that names which.
3. **A refused start reaches the inbox.** `budget.blocked` gets the
   notification the other incidents have, naming the agent, the cap, what was
   observed, and the remedy that already exists — raise the cap or wait for the
   window. A limit that stops work silently is indistinguishable from a broken
   product, and this is the third time it has been read as one.
4. **A start is matched by identity, not by prose.** The CEO is given the
   board's task **ids** alongside their titles, and `"start"` accepts an id;
   a title still works, and still resolves to the most recent open match, but
   the id is what the prompt tells it to use. A paraphrase that matches nothing
   is then a bug in one place rather than a coin toss on every start.

## Consequences

- The CEO's unprompted updates get shorter and truer. A start that lands in the
  inbox now says so, which is also the honest answer to "why is nothing
  running" when the answer is "you have not approved it".
- `docs/THREAT-MODEL.md` is untouched: nothing here changes a boundary. What
  changes is what the product admits about itself.
- The budget gate becomes visible for the first time. Expect it to surface
  incidents that were always there — the owner's own €50 cap has stopped work
  at least three times since 27 August without ever saying so.
- We are committed to the rule that the CEO does not narrate an action it has
  not been told the result of. Any future verb the CEO can invoke — landing a
  diff, opening a PR — inherits it.

## Rejected

- **Making the CEO ask, then answer in a second turn.** Correct and expensive:
  every start would cost another model call, and the CEO already speaks after
  the work is queued. The server knows the outcome the instant it happens; it
  should say so rather than pay for the CEO to ask.
- **Only fixing the budget notification.** It is the loudest of the five
  outcomes but not the only silent one, and the two starts that failed on this
  company never reached the budget gate at all.
- **Refusing a start whose title does not match exactly.** Tempting, and it
  would turn the silence into an error — but it would also break the CEO's
  ordinary flow for a near-miss a person would forgive. Ids first, titles
  still accepted, and the outcome reported either way.
