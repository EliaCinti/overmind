# ADR-0037: Who pays is asked, and Overmind does the switching

- **Date:** 2026-08-23
- **Status:** accepted
- **Builds on:** [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (the economy is detected, never configured), [ADR-0029](0029-the-cage-inside-the-container.md) (commands that run *as the agent*), the M23 sign-in flow (the server drives the CLI's own OAuth so nobody needs a shell).

## Context

The owner ran Overmind natively for the first time on his own Mac — the
Blender company of [ADR-0036](0036-tools-in-the-agents-hand.md) — and the org
chart told him: *you are signed in with a subscription, and it is not paying;
unset `ANTHROPIC_API_KEY` to let the plan pay*. His shell exports the key, the
server inherited it, the CLI's rule is that a key wins over a login. Two things
were wrong with the sentence, and he named both:

1. It was in the wrong place. Who is billed concerns every page, not the one
   with the org chart on it.
2. It told a person to go and edit their environment. The product knew
   exactly what the remedy was and had every means to apply it — and asked the
   person to find a terminal instead. M23 had already settled this for the
   sign-in: *the product offers it, the person approves it, the product does
   it.* A key overriding a plan is the same shape of problem.

The assistant doing the unsetting by hand, on the owner's behalf, was not the
answer either — it is the same terminal, somebody else's hands. The owner's
words: *Overmind should ask, and on approval, do it itself.*

## Decisions

1. **The remedy is a button, and the server applies it.** `POST
   /api/economy/pay-with {"with": "plan"}` makes Overmind keep
   `ANTHROPIC_API_KEY` out of the environment of **every command that runs as
   the agent** — the caged run, the conversational turn, the economy probe,
   the blocking sign-in — and then asks `claude auth status` again. The
   server never calls the API on its own account, so it gives up nothing; the
   CLI, with nothing overriding its login, bills the plan. `{"with":
   "detected"}` withdraws the choice.

2. **Refused when it would be a lie.** If, with the key out of the
   environment, the probe still answers *key* — it lives in a settings file,
   an `apiKeyHelper`, somewhere the environment does not reach — the choice is
   withdrawn on the spot and the request is `409` with the reason. ADR-0030's
   rule stands: a setting that can disagree with who is billed is a setting
   that will, and the disagreement is a bill. The choice therefore only ever
   exists in the state where it is also true.

3. **Remembered as a file, read per spawn.** `<data-dir>/pay-with-plan`,
   presence is the setting — exactly like the stored OAuth token
   (`claude-oauth-token`) and for the same reasons: it survives a restart, it
   needs no migration, and because it is read at each spawn rather than cached,
   withdrawing it takes effect on the next run, not the next boot. The
   startup probe already honours it, so a restarted server reports the plan
   paying without anyone re-choosing.

4. **Said on every page.** `GET /api/health` gains `pay_with: "plan" |
   "detected"`; the top bar carries a three-word badge — *Paying: API key* /
   *Paying: Max plan* — in the warning colour when a key is overriding a
   login. The offer itself is a card at the top of whichever page the person
   is on, beside the sign-in notice, with two buttons: *Let the plan pay* and
   *Keep the key*. The undo sits where the money is read, in the org chart's
   economy line.

5. **Any session may choose.** Like the sign-in flow, this is an
   instance-level fact about the box, and the wall (M24) already limits the
   callers to people the owner invited. No audit event: there is no company to
   write it against, and the server logs the change with the economy it
   produced.

## Consequences

- The word *unset* leaves the interface. A person who reads that their plan is
  not paying is one click from making it pay, and is told plainly when that
  click cannot work.
- Developers with both credentials in their shell — the commonest machine
  Overmind is run natively on — stop paying by accident.
- `OVERMIND_ECONOMY` is unchanged: it *declares*, it does not *switch*. The
  two do not overlap: with an override set the probe is never consulted, so
  choosing the plan under `OVERMIND_ECONOMY=key` is refused (the declared key
  "still pays"), which is the honest answer.
- Tests: `tests/who_pays.rs` (the endpoint, with the economy declared — no
  test spawns the real CLI) and `sandbox::tests` (every agent-side command
  constructor drops the key while the marker exists, and leaves the
  environment alone otherwise).
