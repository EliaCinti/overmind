# ADR-0049: Overmind is the harness, and one sensor is not a loop

- **Date:** 2026-09-06
- **Status:** accepted
- **Builds on:** [ADR-0006](0006-audit-log-and-task-lifecycle.md) (every mutation appends an event), [ADR-0009](0009-heartbeat-scheduler-and-recovery.md) (the heartbeat, and what it was built to carry), [ADR-0023](0023-os-level-sandboxing.md) (every agent run is caged), [ADR-0004](0004-wadachi-first-party-managed-brain.md) (the two-project contract), [ADR-0048](0048-a-provider-is-a-capability-not-a-name.md) (a provider must be stoppable)

## Context

A vocabulary has settled around agents in 2026, and Overmind has been building in it without using it. Naming it costs one document and buys a shared word for two milestones that would otherwise be argued from scratch.

The stack, as the field now describes it: **prompt → context → harness → loop**. Prompt engineering was wording one request well. Context engineering was curating what the model sees before each call. Both meet the same wall — the window fills, quality degrades, and the usual remedy, summarising to make room, buys that room by discarding precision. A **harness** is the scaffolding outside the model that re-initialises the agent step by step: fresh context each step, durable state read back from disk, work resumed where it stopped. A **loop** is what makes a harness autonomous: a goal a *machine* can check, iteration until it is met, and caps that stop it when it is not.

**Overmind is already a harness, and a fairly complete one.** Nothing in this ADR adds scaffolding; it names what is there and finds what is missing.

| Harness concern | Where it already lives |
|---|---|
| Isolation | `sandbox.rs`, `landlock.rs` — one `Confinement` per run ([ADR-0023](0023-os-level-sandboxing.md)) |
| Execution environment | `runner.rs` — a git worktree and a branch per session |
| Tools | `mcp.rs`, `mcp_server.rs` — client and server (M28) |
| Memory | `MemoryProvider` over MCP; Wadachi as first-party ([ADR-0004](0004-wadachi-first-party-managed-brain.md)) |
| Permissions | typed traits, `perm`, `act_within_budget` |
| Model abstraction | the `Provider` trait ([ADR-0048](0048-a-provider-is-a-capability-not-a-name.md)) |
| Observability | the hash-chained audit log, forty event kinds ([ADR-0006](0006-audit-log-and-task-lifecycle.md)) |

**The loop is half built, and the built half is the hard half.** `scheduler.rs` is a heartbeat that recovers orphaned sessions, drains `agent_wakeup_requests`, and lets the CEO write back. Its own opening comment already says what it is for:

> Paperclip's full cron-style routines are deferred; this is the substrate they will sit on.

It has three real stopping conditions, and they work: **time** (`session_timeout_secs`; the child is killed, `Outcome::TimedOut`), **money** (`governance.rs` — reservation, `headroom_cents`, `record_overrun`), and a cap on how many files one run may hand back. A budget cap firing six times in two hours is not a malfunction; it is the loop stopping correctly.

What is missing is narrower than it first looks, and worth stating precisely rather than dramatically.

**One sensor exists.** `runner.rs:1885` is the shape this ADR wants more of: a **knowledge** run that reports `completed` while `delivered == 0` is flipped to `failed` and its task to `blocked`, with the failure worded by the provider rather than by us. It is a deterministic check that overrides the agent's own account of itself, it was measured in the container on 2026-08-15, and its comment already reasons about scope — code runs are excluded on purpose, because "their deliverable is the diff, and one that deliberately changed nothing is a legitimate answer."

**And it is the only one.** Stated exactly:

- No run is checked for **correctness**. One class of run is checked for **presence**.
- A knowledge run that delivers a file saying *"I could not do this"* passes.
- A **code** run is not checked at all. `cargo test`, `npm test`, `run_tests`, `verify`, `lint` — searched across `runner.rs` and `domain.rs` on 2026-09-06: **zero occurrences**. Nothing compiles the branch, nothing runs a test.
- There is **no cap on attempts and no no-progress detection**. `turn_cap` exists, but only for meetings. `step` and `plan` together appear twice in all of `runner.rs`: a task is one session with a timeout, not a loop with a progress check.
- **Forty event kinds are published and nothing subscribes.** The only sources that can wake an agent are `api.rs:2418` (`source = "manual"` — a person clicking) and `meeting.rs:1115` (`source = "meeting"`). The append-only chain is a perfect event bus that nobody reads reactively.

**This is Pillar 1, not a new feature.** `VISION.md` says: *"If we can't prove what an agent did and what it cost, the feature doesn't ship."* The cost is proven — `governance.rs` is rigorous about it. The *doing* is not. For a code run, "done" is currently a sentence the agent wrote about itself, and the ledger records the price of that sentence.

One honest correction belongs in this record, because the first draft of this argument got it backwards. The `NEXT.md` entry *"The CEO reports a deliverable it cannot see"* is **not** an instance of an agent claiming false success. Read in full, the CEO said "its `ARTIFACT.md` is not in the shared folder — I report what it declares, not what it contains" while the file sat at `data/artifacts/<session>/ARTIFACT.md`. The CEO refused to assert what it could not verify, which is the behaviour we want; the bug is that it could not see the file. Overmind has **no recorded case** of an agent inventing success. The argument for sensors is the pillar and the unchecked surface, not an incident — and an ADR that borrowed an incident it did not have would be committing, in prose, the error it exists to prevent.

## Decision

**1. The names are fixed, and written where they are looked for.** Overmind **is the harness**. `ARCHITECTURE.md` names which existing component plays which harness role; `VISION.md` gives the two-project split this vocabulary. The **loop** is the scheduler plus what M35 and M36 add to it.

**2. Two prohibitions keep the two projects separable.** Neither is new policy; both make the existing contract ([ADR-0004](0004-wadachi-first-party-managed-brain.md)) functional rather than merely structural:

- **The memory provider never executes anything and never decides when something starts.** No runner, no cage, no scheduler behind the MCP boundary. Overmind may be pointed at any conforming server; none of them gets to start work.
- **Overmind keeps no long-term knowledge of its own.** What outlives a session belongs to the provider, and Overmind degrades to full functionality without one.

**3. A run is checked, not believed — M35.** Generalise `runner.rs:1885` from one case into a stated contract: a run's own account of itself is evidence, never a verdict. A **code** run is verified by something deterministic the repository already owns (its test command, its build, its linter); a run that fails verification is `failed` and its task `blocked`, worded from the check's own output. The existing exclusion stands where it was reasoned: a diff that deliberately changed nothing is a legitimate answer, and *"nothing changed"* is not the same claim as *"the tests pass"*.

**4. A loop needs a cap it cannot talk its way past — M36.** Attempts are bounded by a count, not only by a timeout and a wallet; a loop that stops making progress is stopped for that reason and says so; and the audit chain gains its first subscriber, so an event can start work the way a person or a meeting already can. Every trigger is attributed in the chain to the event that fired it — an autonomous start with no cause recorded is exactly the start [ADR-0046](0046-a-start-the-ceo-cannot-see.md) refuses.

**5. M35 lands before M36, and this ordering is part of the decision.** Triggers on top of self-reported completion would produce unverified claims made autonomously, unsupervised, and at cost. The only brake standing today is the wallet, and a spending cap is not a correctness criterion — it limits how much a wrong answer costs, not whether it is wrong.

## Alternatives considered

**Triggers first.** It is the more visible half and the one the owner felt: tasks that will not start without a click. Rejected on ordering only — see decision 5. It ships immediately after M35, not instead of it.

**An LLM judging the run.** Cheap to build and applies to every task kind, including knowledge work where no test exists. Rejected as the *primary* signal: a verifier the agent can argue with is one it can argue past, and Overmind's accountability claims are held by tests, not by opinions (`CLAUDE.md`). It stays available as an *advisory* second opinion, never as the thing that flips a status.

**Putting a loop in the memory provider.** Wadachi's `reflect`, `sleep` and `consolidate` are loop-shaped and might have been extended into one. Rejected: they are background maintenance that *proposes* and never acts, the provider executes nothing, and building a loop on both sides of the MCP boundary would mean two loops that can disagree about whether work is finished.

**Leaving the vocabulary implicit.** The code would be identical. Rejected because the roadmap has already paid this bill once: M32 and M33 were opened with ADRs and PRs and were absent from `ROADMAP.md`, so there was nowhere to look to find out where we were.

## Consequences

- **M35 and M36 have a shared vocabulary and a stated order**, and each has an acceptance criterion that comes from Pillar 1 rather than from taste.
- **`completed` stops meaning what it means today.** A run can now fail *after* the agent believed it was finished. That needs an event kind of its own, a task state a person can act on, and a UI that distinguishes "the agent gave up" from "the check said no" — a distinction the board does not currently draw.
- **Verification costs time and can be wrong.** A flaky test suite becomes a way to block a task that was fine. M35 must decide what happens when the check itself fails to run, and the honest default is to say so rather than to pass.
- **A knowledge task has no `cargo test`, and this ADR does not solve that.** `ExecutionKind` has two variants, and the loop pattern that works so well on code works because a machine can grade it. What counts as a deterministic check for `Knowledge` is an open question, owned by M35, and named here so it is not discovered late. `empty_handed` is the floor for that case, not the answer.
- **We are committed to never deriving "done" from a self-report alone**, and to never letting the memory provider start work. Both are testable, and M35 and M36 carry the tests.
