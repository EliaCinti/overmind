# ADR-0030: How you pay is a first-class fact, and the cap promises only what it can

- **Date:** 2026-08-17
- **Status:** proposed

## Context

Since [ADR-0012](0012-budgets-and-governance.md) Overmind has had one economic
model: an agent has a monthly cap in cents, a run reserves against it, and
[ADR-0022](0022-conversational-spend-under-budget.md) finished the job by making
every adapter invocation record what it cost. That model is correct, tested, and
quietly assumes something nobody wrote down — **that spending is measured in
money**.

It is, if you hold an API key. It is not, if you hold a subscription. Under a
plan there is no dollar to run out of. There is a window, a quota inside it, and
a refusal when you reach the end of it. `spent_cents` under a subscription is
not wrong so much as beside the point: it is a number the CLI computes as
*equivalent* cost, against a cap that corresponds to nothing you will ever be
charged.

Both are ways people actually run this. The image now supports both — M19 gave
the agent a home whose credentials survive a rebuild, which is what makes
`claude setup-token` viable in a container — so the question is no longer
whether Overmind can be run on a plan, but what it is entitled to *say* while
you do.

### What was measured before designing anything

With claude 2.1.233, on 2026-08-17:

- **The economy is detectable.** `claude auth status --json` returns
  `authMethod` and `apiKeySource`, free and local. Clean where there is one auth
  source — inside the container it answers `api_key` — and **ambiguous where
  there are two**: on a host holding both a claude.ai login and a key, it
  answered `authMethod: "claude.ai"` *and* `apiKeySource: "ANTHROPIC_API_KEY"`,
  with `subscriptionType: null`.
- **A key silently outranks a plan.** With both present the CLI warns:
  *"claude.ai connectors are disabled because ANTHROPIC_API_KEY or another auth
  source is set and takes precedence over your claude.ai login"*. Preference is
  not how you choose a plan; **absence of a key** is.
- **There is a ceiling we are not using.** `--max-budget-usd`, which works only
  with `--print` — the mode Overmind invokes — and fails recognisably:
  `subtype: "error_max_budget_usd"`, `terminal_reason: "budget_exhausted"`. It
  **overshoots**: a $0.05 cap recorded $0.080729, because it stops after
  exceeding rather than pre-authorising.
- **Plan quota is not on the surface we consume** — which is a narrower claim
  than the one this ADR first made, and the correction is recorded below.

That last one is the whole difficulty. The natural feature request — *"under a
plan, show me what is left in the window"* — asks for a number that is not in
anything Overmind reads.

### Correction (2026-08-17, same day): the quota exists, on a surface we do not use

This ADR first said the quota was "exposed nowhere we can reach". That was
wrong, and it was wrong in the way worth catching: an absence inferred from the
two places we happened to look. Reading the CLI's own vocabulary rather than
guessing at it turns up a documented contract — the JSON a **status line**
command receives:

```
"rate_limits": {   // Optional: Claude.ai subscription usage limits.
                   // Only present for subscribers after first API response.
  "five_hour": { "used_percentage": number, "resets_at": number },
  "seven_day": { "used_percentage": number, "resets_at": number }
}
```

A sibling field says plainly when it does not apply: `rate_limits_available` is
*"False when plan rate limits do not apply (API key / 3P provider sessions)"* —
which independently confirms the economy split this ADR is built on.

**It changes the reason, not the decision.** A status line belongs to an
interactive session, and Overmind invokes `-p`; the headless result envelope
carries `usage`, `modelUsage`, `service_tier` and `total_cost_usd`, and no quota.
So the life-line still shows no denominator under a plan — but because the
number is absent from *the surface we consume*, which is a fact about our
invocation and could change, rather than because it does not exist, which would
have been a claim about the world that happened to be false.

Exhaustion has a shape too, and it is worth writing down now that it has been
seen:

```
error: { message, status, formatted, is_network_down,
         rate_limits: { resets_at?, rate_limit_type? } | null
           // "Quota-429 headers surfaced by the retry banner;
           //  null when not a quota 429." }
```

So a plan running out is a **quota 429 carrying a reset time**, not one of the
adapter's `subtype` values — the closed set is `success`, `error_during_execution`,
`error_max_turns`, `error_max_budget_usd`, `error_max_structured_output_retries`,
and none of them is this. That is why exhaustion cannot be recognised the way a
budget ceiling can, and it is marked `@internal` on a retry event rather than
present on the result. What reaches a headless run is whatever `formatted`
became. **Still not matched on**, because the string has not been observed, and
a matcher written from a schema is a matcher written from a guess.

## Decision

**The payment economy is a first-class property of a running Overmind**, and the
interface says which one it is in, what the cap means there, and where a number
would be a guess.

### 1. Detected, stored, and overridable — in that order

Asked of `claude auth status --json` at startup and cached; surfaced in the UI
beside the budget. Detected rather than configured, because a setting that can
disagree with reality is a setting that will: the person who exports a key into
a shell that already had a subscription would be looking at whichever answer
they typed six weeks ago.

Where the answer is ambiguous — both sources present — we take the CLI's own
precedence rule and call it **key**, because that is what will actually be
billed. An explicit override exists for the case we read wrong, and it is a
declaration of what you believe, not a switch that changes behaviour.

### 2. With a key: the cap becomes a ceiling, not only a ledger

The remaining budget is passed as `--max-budget-usd` on every invocation. The
M6 gate stays exactly what it is — the decision to spend is a human's, checked
before the run — and this is the second layer *inside* the adapter, where M18's
stated gap lives:

> a turn is still not priced before it runs — the estimate is flat, as it is for
> tasks, so an agent can overrun by the difference between the estimate and one
> turn's true cost.

This does not close that gap; it narrows it from *a whole turn* to *the
adapter's own overshoot*. Recorded as a narrowing, not a fix, because claiming
the second would be the mistake this project keeps finding in its own past —
and because the overshoot turns out to be large:

| ceiling given | actually spent | over by |
|---|---|---|
| $0.05 (bare probe) | $0.080729 | 1.6× |
| $0.05 (a real task through Overmind) | $0.13 | 2.6× |

It gets proportionally **worse** the smaller the ceiling, because a run's fixed
cost — the context loaded before a single useful token — does not shrink with
it. So this is a coarse brake and must be described as one: it will stop a
runaway agent long before a flat estimate would, and it will not hold anyone to
a number. A cap of a few cents is not a cap of a few cents.

Two consequences follow, and both are deliberate. The ledger records what was
truly spent, not the ceiling, so an agent can end a run *over* its cap — and the
next gate check then refuses it, which is the correct behaviour and would be
wrong to smooth over. And it is passed in **every** economy, not only under a
key: the cap is also the brake on a looping agent, a plan's quota is exactly
what a loop burns, and the brake is the same brake whether or not the number
means dollars. That generalises what this ADR first proposed, on the ADR's own
reasoning that the cap survives and only its meaning is restated.

`error_max_budget_usd` is recognised and reported as what it is — the agent hit
its ceiling — rather than as a generic adapter failure. A person who is told
"the run failed" learns nothing; a person told "it stopped at your cap" knows
what to do.

### 3. With a subscription: our own consumption, named as ours

**No invented residual.** Overmind knows precisely what *it* spent in the
window, because every adapter invocation has recorded a `cost_event` since M18.
It does not know what your plan has left, because you use the CLI outside
Overmind — including, on the machine this was written on, to write Overmind.
Showing the first under the second's name would put a fourth entry in the family
of `permissions`, `model` and the cost ledger: believed, displayed, and wired to
something that is not what it claims.

### 3b. Amended the same day: the plan *does* report, and it reports two clocks

Pushed to look harder rather than accept "no", the answer changed twice. A
headless run **does** report on the plan — not in the `-p json` envelope, but as
a `rate_limit_event` on `--output-format stream-json --verbose`, measured:

```json
{"type":"rate_limit_event","rate_limit_info":{
  "status":"allowed","resetsAt":1786983000,"rateLimitType":"five_hour",
  "overageStatus":"rejected","overageDisabledReason":"out_of_credits",
  "isUsingOverage":false}}
```

So the default adapter command becomes `stream-json`, and the reason is not the
streaming: it is that the plan's state then **rides along with work already
being done** rather than costing a call of its own — the bargain ADR-0026 made
for memory watermarks. The final `result` event is the identical envelope
`json` produced, so cost parsing and failure reporting read exactly what they
always did.

What this gives, and what it still does not:

- **Not a percentage.** `used_percentage` lives only in the status line, and a
  status line is never invoked headless — measured directly, with a probe
  configured to capture it, which never fired.
- **But a window, a reset time, and a state.** Which of the plan's two clocks is
  limiting, when it lets go, and `allowed` / `allowed_warning` / `blocked` /
  `rejected` — the adapter's own closed vocabulary, read rather than inferred
  from prose. For someone waiting on a plan, "the five-hour window is back in
  two hours" answers the question a percentage only implies.
- **Two clocks, kept apart.** A plan limits on five hours *and* seven days, and
  a run reports whichever is biting at that moment. So they are learned
  separately and displayed separately: collapsing them into "the plan" hides
  the one you are about to hit. A window nobody has reported yet says **"not
  reported yet"** rather than borrowing the other's state — "we have not heard"
  and "you are fine" are different sentences, and only one of them is true
  before the first report.

**Exhaustion is therefore recognisable exactly**, from `status`, and not by
matching prose. Whether a room *pauses* on it is the next question and is not
settled here; what is settled is that the signal exists and has a name.

The correction that made this possible is worth keeping in view: the first
answer was "the quota is exposed nowhere", and it was an absence inferred from
two places we happened to look.

**Exhaustion is recognised when it arrives.** M18 already put this on the shelf:
*"subscription exhaustion is a different failure that arrives as an adapter
error; the pause path is where it belongs once we can recognise it reliably."*
A room that runs out of plan pauses exactly as a room that runs out of budget
does — the machinery exists, it needs a second thing to recognise. Until a real
exhaustion is observed and its shape written down, it surfaces as an adapter
failure, and that limitation is documented rather than guessed at.

### 4. A key that outranks a plan is said out loud

If a key is present, Overmind says so where the budget is shown, in one line.
The CLI already warns; a warning in a log nobody reads is not the same as the
interface telling you that the plan you signed into is not the thing paying.
Half of this landed in M19, when the compose file stopped defining
`ANTHROPIC_API_KEY` as empty-but-present in every container.

## Alternatives considered

- **A configured setting instead of detection.** Simpler, and wrong in the one
  case that matters: it can disagree with reality, silently, and the disagreement
  is a bill.
- **Show a token count with no window, under a plan.** Tokens are visible per
  run; a raw running total without the window boundary invites the reader to
  compare it against a limit they half-remember. The window is the honest unit,
  and we know when it started only for our own spend.
- **Estimate the plan's remaining quota** from published limits and observed
  usage. Rejected outright: it would be a guess wearing a number's clothes, and
  wrong in the direction that costs someone a working afternoon.
- **Anthropic's Admin usage-and-cost API.** Real data, but it needs a separate
  admin key and answers a question about an *organization*, not about the plan
  behind one CLI login. A different product, noted so it is not rediscovered as
  an oversight.
- **Drop the cap under a subscription.** Tempting — no money is at stake — and
  rejected: the cap is also a brake on a looping agent, and a plan's quota is
  precisely the thing a loop burns. The cap survives; only its *meaning* is
  restated.

## Consequences

- **Two economies mean two sets of words** in the interface. The alternative is
  one set that is right in one of them.
- **The cap keeps its units.** Cents, in both, because that is what the adapter
  reports and what the ledger holds. Under a plan the number is an equivalent
  rather than a charge, and the interface has to say so wherever it appears.
- **`--max-budget-usd` is a per-invocation flag**, so it must be computed per
  invocation from the remaining budget. A stale ceiling is worse than none: it
  would let a run spend against a cap that had already been consumed.
- **Detection costs a subprocess at startup.** Cached, and not fatal when it
  fails: an Overmind that cannot tell which economy it is in says that, rather
  than assuming the one that bills you.
- **An open question stays open**, deliberately: what a subscription's
  exhaustion looks like on the wire. It cannot be produced on demand without
  exhausting somebody's plan, so it is recognised when first observed, and the
  code says as much where it would go.
