# ADR-0050: A loop that converges, and a stop that is not a crash

- **Date:** 2026-09-07
- **Status:** proposed — for review before any code
- **Builds on:** [ADR-0049](0049-the-harness-and-the-loop.md) (Overmind is the harness; M35 and M36 are its loop), [ADR-0012](0012-budgets-and-governance.md) (budgets, governance and the `requires_approval` gate), [ADR-0022](0022-conversational-spend-under-budget.md) and [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (the wallet as a brake), [ADR-0042](0042-the-ceo-runs-the-floor.md) (dependencies release work), [ADR-0046](0046-a-start-the-ceo-cannot-see.md) (an autonomous start without a cause is refused), [ADR-0048](0048-a-provider-is-a-capability-not-a-name.md) (a provider must be stoppable)

## Context

**What the owner saw.** One company, TravelAgency, run for real on the native install between 26 August and 6 September 2026. Counted in its database and its server log on 7 September:

| | |
|---|---|
| runs | **94** — 67 completed, **27 failed** (29%) |
| `budget.blocked` events | **25** |
| tasks | 75 created — **67 sitting in `in_review`**, 4 `todo`, 3 `blocked`, 1 cancelled |
| approvals requested | **62**, for 75 tasks — a human gate on more than four tasks in five |
| agent-to-agent wakeups | **19**, against 653 artifacts produced |
| identical refused starts in the log | **17** — the same task, the same agent, the same correct refusal, thirty seconds apart |
| agent turns dead on `exit 1` | **16**, nearly all the provider's sign-in or its limits |

The owner's words for it: *the tasks still depend on me too much; the agents do not coordinate; it is always me who moves a task into "to do"; and the system must never stop dead in the middle of work.* The numbers say the same thing three ways. The human is the gate on nearly everything (62 approvals, 67 reviews), so the floor waits for one person. The agents barely talk to each other (19 wakeups). And when something goes wrong, it goes wrong *again*.

**The 17 refusals are a bug, and the bug is the argument.** Traced on 7 September: the multimodal gate returns `RunnerError::Remediable` (`runner.rs:779` — a task carrying an image, an agent not characterized for visual work, and the remedy Overmind could apply itself: one traits patch). `wakeup_outcome` in `scheduler.rs` turns `Conflict`, `OverBudget`, `Invalid` and `Blocked` into a recorded outcome and lets everything else fall through as an error; `process_wakeups` propagates that error with `?` *before* the row is marked `done`, so the wakeup stays `queued`, the beat aborts — starving every later wakeup and the digests behind it — and the next heartbeat, thirty seconds on, re-selects the same row and re-refuses the same task. Seventeen times is eight and a half minutes. The fix is one match arm, and it ships on its own, before this ADR is even accepted. What this ADR takes from it is the shape: **a harness that let one refusal repeat seventeen times without noticing is a harness with no progress measure**, and a bug of that shape will happen again, in a place no one has traced yet. The detector below is defence in depth; it is not how this bug gets fixed.

[ADR-0049](0049-the-harness-and-the-loop.md) named the harness and found the loop half built. To be exact about the baseline, since this ADR proposes to change it: a run stops today on **time** (`session_timeout_secs`, `Outcome::TimedOut`), on **money** and on **volume** (the cap on files a run may hand back). The money brake acts twice: `governance.rs` refuses a turn *before* it starts when `spent + reserved + estimate` would pass the cap, emitting `budget.blocked`; and the provider's own per-run bound (`--max-budget-usd`) kills a run *mid-turn*. There is one sensor (`empty_handed`), no cap on **attempts**, no progress detection, and forty-one event kinds with no subscriber. ADR-0012 deferred an *80 % warning* and never returned to it. This ADR does not reopen ADR-0049. It reads what the field converged on during 2026 and decides how Overmind's loop closes — and, as important, how it *stops*.

**What the field says, read on 7 September 2026.** Sources at the end; the principles below are the ones that appeared independently in three or more of them. The owner's own reference — Simone Rizzo's account of loop engineering — is the same shape in one sentence: *give the agent a clear, verifiable goal and let it iterate until it is met, instead of babysitting each step.* His worked case is worth carrying whole, because it is the loop that Overmind's code runs are missing: a matrix-multiplication script, the goal *"make it faster, ten attempts, document each change"*, and an agent that moved to PyTorch, then CUDA, then tensor cores — **320× faster, no human in the loop** — because the goal was a **number the machine measures**, the attempts were **bounded**, and every step **wrote down what it did**.

| Principle | Where it comes from | Overmind today |
|---|---|---|
| A loop converges only with **a progress measure that is external**, **hard bounds**, and **designed exits**. "A loop without a bound isn't autonomous, it's unattended." | Munder Difflin; Anthropic harness; Huntley; Rizzo | bounds in money, time and volume; progress unmeasured; exits are *done*, the wallet, the clock |
| **The goal is something a machine checks** — a test that passes, or a metric that moves in the right direction — and the loop iterates against it within a count of attempts. | Rizzo; Huntley; Anthropic | a task has a description, not a check |
| **The generator does not grade itself.** "Agents tend to respond by confidently praising the work — even when the quality is obviously mediocre." Evaluator separate, hard threshold per criterion. | Anthropic harness design | one sensor, presence only (M35 owns this) |
| **State lives outside the model** — a plan file, specs, git; re-read first thing on every restart. One item per loop; each step documented. | Huntley (Ralph); Anthropic; Osmani; Rizzo | worktrees, artifacts, the audit chain, Wadachi desks |
| **Three currencies, none a substitute for the others**: steps, tokens/money, wall-clock. Propagate an **absolute deadline**, not durations. | Konishi | money and time; **no step budget** |
| **Three detectors, not one**: exact repetition (fingerprint of tool + arguments), stagnation (a progress predicate on durable state — "anything the agent asserts about itself is excluded"), cycling (repeat of a seen state). **Graduated response**: inform → constrain → stop. | Konishi; Waxell; Munder Difflin | none |
| **A retry is not a repeat.** Before retrying a step with external effects: compensate, reconcile, or refuse — recorded per tool, next to its schema. "Failed once" and "failed the same way three times" are different facts. | Konishi; OpenAI | retries are the CEO relaunching; effects not classified |
| **Advertised budget vs enforced ceiling.** Show the model a budget it can wind down under; enforce a higher one that fires only when the wind-down misbehaves. Reserve a slice for the ending: checkpoint, partial result. | Konishi | the pre-turn refusal exists; the mid-turn bound kills |
| **Escalation is a designed exit with a packet**: goal verbatim, side-effect ledger, what is pending, why it stopped, what it proposes, the one decision needed, a resume id **and a deadline**. "A pending approval is a piece of state with an owner and a deadline, not an open socket." | Konishi; 12-factor (#7) | `approval.requested` carries a task, not a packet |
| **Approve by reversibility, not by default.** Irreversible or high-value actions get a human "until the agent's reliability is proven"; the rest run. Evaluated by policy *before* execution, never left to the agent's judgment. | OpenAI; Konishi | `requires_approval` gates **every** start of that agent |
| **Share context, not just deliverables.** "Actions carry implicit decisions, and conflicting decisions carry bad results." | Cognition | a dependency inherits the deliverable, not the reasoning |
| **Trigger from anywhere; launch, pause, resume as primitives.** | 12-factor (#6, #11) | manual and meeting only (M36 owns this) |
| **The ratchet**: every failure becomes a permanent rule. "The right harness for your codebase is shaped by your failure history." | Osmani | the audit chain records failures; nothing turns them into rules |

One thing every source agrees on and none of them solves for us: the loop works on code because a machine can grade code. What a deterministic check is for **knowledge** work remains M35's open question, and this ADR keeps it open rather than pretending an LLM grader closes it (rejected as primary signal in ADR-0049).

## Decision

**1. Three currencies, one deadline.** Every task carries a bound in **attempts**, **money** and **wall-clock**, and a run inherits an *absolute* deadline computed when the task is admitted — each layer clamps to the time remaining, so nested retries cannot multiply past it. `Bound` in `provider/mod.rs` has one variant today, `Money`, and ADR-0048 already reserved the second, a count of turns; this decision is what makes it grow — `Turns`, and a deadline — and the task speaks the same type as the provider.

**2. A task may carry a check, and the loop iterates against it.** This is Rizzo's loop and Huntley's, stated as a contract: a task may declare **how it is checked** — a command the repository owns, read either as *pass/fail* (a test suite) or as *a number and a direction* (a benchmark: lower is better) — and **what done is** (the tests pass; the number crosses a target; or simply the best of `N` attempts). Each attempt runs in the same worktree, keeps its change only if the check improved on the previous best, and **writes down what it did and what the check said** — the record is Overmind's, not the agent's word for it. The check itself is M35's deliverable and the reason M35 exists; what this decision adds is that a *metric* and a *verdict* are the same mechanism with a different comparator, and that a task without a declared check gets `empty_handed` as its floor and nothing more.

**3. Progress is measured on durable state, never on the agent's account of itself.** Three detectors, each with a window, a tolerance and a graduated response, each rung an audit event of its own. Two of them need the check of decision 2 to have anything to measure, and that fixes their order (decision 9):

- **repetition** — a fingerprint of (agent, task, cause) for a refused start, and of (tool, arguments) for a call; the same fingerprint `N` times in a row trips it. *Needs no verification, only the audit chain, and would have caught the scheduler bug at the third beat.*
- **stagnation** — the check of decision 2 not moving while spend rises: the diff and the verdict for a code run, the verified deliverables for a knowledge run. *After M35.*
- **cycling** — a fingerprint of the task's observable state; a state seen before is a cycle. *After M35.*

Response: **inform** (the observation is injected into the run's next context: *"this refusal has been given three times with the same reason"*), then **constrain** (the repeated action is withdrawn from what the run may do), then **stop** — and a stop is decision 4, not a kill.

**4. A stop is a wind-down, not a crash.** Two numbers per bound: the **advertised** one, which the run sees and is expected to finish under, and the **enforced** one, above it, which fires only when the wind-down itself misbehaves. A slice of the deadline is reserved for the ending. A run that reaches its advertised bound writes a **handoff**: what was done, what the check said, what is pending, what it proposes next — a Wadachi desk when memory is on, Overmind's own record when it is not — and its task lands in a state a person can read, `stalled`, with the reason in the provider's words. **`budget.low`** is the warning ADR-0012 deferred at 80 % and never delivered, now with the agent's name in it, emitted when the headroom falls under the learned cost of the next turn — before `governance.rs` refuses the turn, which it already does correctly and silently. Nothing that reaches a bound is allowed to leave the task with no account of itself: that is the sentence behind *"it must never stop dead in the middle of work"*, and it is testable.

**5. Escalation is a designed exit, with a packet and a deadline.** `approval.requested` and the new `task.stalled` carry the packet from the table above, verbatim goal to resume id. A pending approval has an owner and a deadline, and the deadline is a fact the CEO can plan around, not a socket left open. **Approval is calibrated by reversibility, not granted by default.** The gate today is the agent's `requires_approval` flag ([ADR-0012](0012-budgets-and-governance.md)), which files an approval on **every** start of that agent; actions are classed in the tool registry and by task kind — *reversible* (run), *compensable* (run, with the compensation recorded next to the effect), *irreversible or high-value* (a human, before, until reliability is proven and the owner widens the envelope) — and `requires_approval` narrows from "every start" to "the irreversible ones". A second lever is named so it is not confused with the first: agents with autonomy `act_with_approval` are never woken by the scheduler at all (`scheduler.rs:234`), so autonomy for them is a hiring choice, not a gate. And the **review queue** is for what needs a person: a run that passed its check (M35) and touched only reversible state is accepted with a line in the digest, not parked in `in_review` — 67 of them was the queue saying it was misused.

**6. A wakeup carries the reasoning, not only the deliverable.** When a dependency releases a task ([ADR-0042](0042-the-ceo-runs-the-floor.md)), the dependent inherits the predecessor's **handoff** from decision 4 alongside its files — written by the run as part of finishing, so it costs no extra turn. ADR-0040's compaction is for chat threads and is a paid turn of its own; it is *not* reused here. One hop: a dependent gets its predecessor's handoff, not the chain's. This is the whole of what Cognition asks.

**7. Triggers subscribe to the chain, after M35, and every trigger is attributed.** Unchanged from [ADR-0049](0049-the-harness-and-the-loop.md) §4: the first subscriber of the audit chain; a task may wait for an event kind, not only for a task; and every autonomous start names in the chain the event that fired it ([ADR-0046](0046-a-start-the-ceo-cannot-see.md)).

**8. The ratchet writes memory.** Every breaker stop and every escalation stores what tripped it and why, as a memory in the company's brain, so that the next hire, the next task prompt and the next envelope are shaped by this company's failure history and not by defaults. The memory provider still starts nothing (ADR-0049 §2); it only remembers.

**9. The order changes, and ADR-0049 §5 is amended by one distinction.** §5 forbade *autonomous starts* on top of self-reported completion, and that stands. But an attempts cap, a repetition detector, a wind-down and a warning are **brakes**, not starts: they make an unverified loop *shorter*, never longer. So the sequence is **M36a — the brakes**: decision 1, the repetition detector of decision 3, decision 4 and `budget.low`; then **M35 — the check**: decision 2, and the stagnation and cycling detectors it makes possible; then **M36b — the triggers**: decisions 5, 6, 7, 8. M36a's acceptance criterion presupposes nothing of M35: *a refusal repeated to its tolerance stops the task as `stalled` with the reason; a run reaching an advertised bound leaves a handoff and never an empty task; `budget.low` names the agent before the wallet says no.*

**10. The numbers are provisional, and the ledger re-derives them.** Initial defaults, stated so they can be wrong in public: attempts **3** per (task, cause); repetition tolerance **3**; advertised budget at **85 %** of the enforced one; **10 %** of the deadline reserved for wind-down. After the first week of real use, the distribution of *successful* runs is measured and the bounds set above its tail with headroom — the procedure the reliability literature gives, applied to our own ledger. A rising escalation rate is read as *the envelope is too wide or a capability is missing*, not as agents misbehaving.

## Alternatives considered

**An LLM grading progress.** Rejected in [ADR-0049](0049-the-harness-and-the-loop.md) as the primary signal and rejected again here: the detectors of decision 3 are deterministic on purpose, and the field's own warning is that the model praises its own work.

**The circuit breaker as a kill.** The pattern most written about terminates on the first trip and writes an audit record. Rejected for Overmind: a kill throws away the partial work and the account of it, which is exactly what the owner cannot afford. Graduated response and wind-down cost more to build and are the difference between *stopped* and *lost*.

**A detector instead of a bug fix.** Building the repetition detector *as* the fix for the scheduler's missing arm. Rejected: the bug is one line, and infrastructure layered over a bug is a bug with a roof. The detector is for the next one.

**Approval on everything.** The status quo in TravelAgency — 62 approvals for 75 tasks — and the numbers reject it: a gate on everything is a gate on nothing, because the person becomes the bottleneck and the queue fills with what they cannot read.

**No gates.** Rejected: there are irreversible actions on the floor (a message sent, a payment, a deletion), and the literature is unanimous that those meet a person until reliability is proven.

**Reusing chat compaction as the trace between tasks.** Rejected: it is chat-scoped, it is a paid turn per release, and a chain of dependents would either nest summaries or drop them. The run's own handoff is free and already required by decision 4.

**A durable-execution engine (Temporal and kin).** They solve resume-where-you-stopped well. Rejected: Overmind already owns its control flow (12-factor #8), the audit chain and SQLite *are* its durable log, and the default retry policy those engines ship was itself reported as a retry-storm source on rate limits.

**Fixed caps, forever.** Rejected in favour of decision 10: a number nobody re-derives is a number that is wrong for the next model.

## Consequences

- **New audit event kinds** in `domain.rs::event_kind`: `loop.informed`, `loop.constrained`, `task.stalled`, `task.wound_down`, `trigger.fired`. **One new notification kind** in `notify.rs::kind`, beside `budget.exhausted`: `budget.low` — a notification because a person acts on it, and an audit event too, because it is a fact about the run.
- **A task state a person can act on**: `stalled` sits beside `blocked` and means *the loop stopped it, and here is why* — the UI draws the distinction ADR-0049 asked for between "the agent gave up" and "the check said no". A check that fails to run (a flaky suite, a missing toolchain) is M35's case and lands in `blocked` with the check's own output, as ADR-0049 §3 already says; `stalled` is reserved for the loop.
- **A task gains an optional check** (decision 2): a command, a comparator, a target. The examples in `docs/examples/` show one of each kind.
- **The registry gains a column**: reversibility per tool, next to its schema.
- **`requires_approval` changes meaning.** Existing agents keep the flag; what it gates narrows from every start to the irreversible ones. This is a behaviour change for running companies and ships with a line in the digest saying so.
- **`Bound` grows** from one variant to a type with attempts, money and a deadline, shared by providers and tasks.
- **Verification can be wrong, and a bound can be too tight.** The first lands in `blocked`, the second in `stalled`, each with the reason, which is the honest answer; decision 10 is how the bounds get better.
- **Knowledge work still has no `cargo test`.** Unchanged, owned by M35, named here so decision 3's stagnation predicate for knowledge runs is understood as *delivered and verified*, with `empty_handed` as the floor.
- **This is Pillar 1, again.** *If we can't prove what an agent did and what it cost, the feature doesn't ship.* Today the cost is proved; after M36a the *stopping* is proved; after M35 the *doing* is.

## Sources, read 7 September 2026

- Simone Rizzo — the owner's reference on loop engineering (video, summarised by the owner on 7 September; channel [@simone_rizzo98](https://www.youtube.com/@simone_rizzo98)): a verifiable goal, bounded attempts, self-documented steps; the matrix-multiplication case, 320× without a human.
- Anthropic — [Harness design for long-running application development](https://www.anthropic.com/engineering/harness-design-long-running-apps): planner / generator / evaluator, hard thresholds per criterion, the self-praise failure mode, structured handoffs across context resets.
- Addy Osmani — [Agent Harness Engineering](https://addyosmani.com/blog/agent-harness-engineering/) and [Long-running Agents](https://addyosmani.com/blog/long-running-agents/): durable state, verification and recovery, the ratchet.
- Geoffrey Huntley — [Ralph](https://ghuntley.com/ralph/): state in files, one item per loop, tests as the feedback, "engineers are still needed".
- Cognition — [Don't build multi-agents](https://cognition.com/blog/dont-build-multi-agents): share context and full traces; actions carry implicit decisions.
- HumanLayer — [12-factor agents](https://github.com/humanlayer/12-factor-agents): contact humans with tool calls (#7), own your control flow (#8), launch/pause/resume (#6), trigger from anywhere (#11).
- OpenAI — [A practical guide to building agents](https://cdn.openai.com/business-guides-and-resources/a-practical-guide-to-building-agents.pdf): retry limits then escalate; human oversight for irreversible actions until reliability is proven.
- Hidekazu Konishi — [Agent Reliability Engineering Design Guide](https://hidekazu-konishi.com/entry/agent_reliability_engineering_design_guide.html): three currencies, absolute deadlines, three detectors, advertised vs enforced budgets, the escalation packet, compensate / reconcile / refuse.
- Waxell — [AI Agent Circuit Breakers](https://www.waxell.ai/blog/ai-agent-circuit-breaker-pattern) and Munder Difflin — [Loop Engineering: Designing Agent Loops That Converge](https://munderdiffl.in/blog/loop-engineering-for-ai-agents/): trip conditions, "a loop without a bound isn't autonomous, it's unattended", inform → constrain → stop.
