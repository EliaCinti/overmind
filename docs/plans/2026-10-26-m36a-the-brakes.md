# M36a — the brakes

- **Date:** 2026-10-26
- **Decided in:** [ADR-0050](../adr/0050-a-loop-that-converges-and-a-stop-that-is-not-a-crash.md) (decisions 1, 3, 4, 9, 10), [ADR-0049](../adr/0049-the-harness-and-the-loop.md) addendum (a brake is not a start)
- **Status:** approved by the owner, 2026-10-26

## The problem, in the code as it is

A brake makes an unverified loop shorter, never longer, so M36a ships before M35. What it has to brake, read on `main` at `2bbc406`:

- **A timed-out task is retried for ever.** `runner.rs` sends `Outcome::TimedOut` back to `todo`, and `Outcome::Infra { release: true }` too. Nothing counts. The next wakeup's `SELECT … WHERE status = 'todo' ORDER BY created_at` picks the same task first, it times out again, and the loop is bounded only by the wallet — the brake ADR-0049 §5 says is not a correctness criterion.
- **A refused start repeats without being noticed.** After #107 a refusal no longer stalls the beat: it is recorded in `agent.wakeup_processed` and the wakeup moves on. But it is recorded as a sentence (`"refused: task … refused: …"`), and the next wakeup refuses the same task for the same reason. Nobody counts that either; the 17 refusals of TravelAgency would now be 17 audit rows instead of a stalled beat.
- **A run that hits a bound leaves nothing.** `TimedOut` kills the child; the provider's `--max-budget-usd` ends it mid-turn. Either way the task gets `last_error` and no account of what was done.
- **The budget warns too late.** `governance.rs` refuses a turn when it would not fit, correctly and silently; `budget.exhausted` arrives after. ADR-0012's 80 % warning was never built.

## The approach — four slices, one PR each, in this order

Every slice starts with its test, watched failing for the expected reason (CLAUDE.md). The numbers are ADR-0050 §10's and live in `Config`, so the ledger can re-derive them later without a code change.

### 1. `stalled`, and the attempts cap

- `TaskStatus::Stalled` (`"stalled"`). `tasks.status` is `TEXT` with no `CHECK`, so no migration for the value itself. Transitions: `Todo → Stalled`, `InProgress → Stalled`, `Stalled → Todo` (a person restarts it), `Stalled → Cancelled`. The scheduler never picks a `stalled` task.
- **An attempt** is a session of the task that ended `failed`; its **cause** is the outcome class (`timed_out`, `infra`, `agent_failure`, `empty_handed`). At the third failure with the same cause (`attempts_cap = 3`), the task goes to `stalled` instead of `todo`/`blocked`, with the reason in words: *"timed out 3 times (7200 s each)"*. Counted from `agent_task_sessions`, the durable state — not from anything the agent said.
- `task.stalled` audit event carrying the packet: cause, attempts, the last error, the session ids.
- UI: `stalled` drawn apart from `blocked` (ADR-0049: "the loop stopped it" ≠ "the check said no"), with the reason and a *Restart* action. Strings through `i18n.rs` / the web dictionary.
- **Tests first:** `a_task_that_times_out_three_times_stalls_instead_of_looping`, `a_different_cause_does_not_add_to_the_count`, `a_person_restarting_a_stalled_task_resets_its_count`.

### 2. The repetition detector on refused starts

- `agent.wakeup_processed` gains `refusals: [{task_id, cause}]`, the cause being the `RunnerError` variant plus, for `Remediable`, the remedy's kind — data, never the sentence (the contract #107 put on remedies).
- Fingerprint `(agent, task, cause)`; the same fingerprint at `repetition_tolerance = 3` consecutive wakeups → the task goes to `stalled` with *"refused 3 times: <reason>"*, and the remedies travel in the `task.stalled` packet so the owner sees what would unblock it. A different outcome in between resets the run of repetitions.
- Read from the audit chain only (ADR-0050 §3: *needs no verification, only the audit chain*).
- **Tests first:** `a_start_refused_three_times_for_the_same_reason_stalls_the_task_with_its_remedy` — the TravelAgency case, multimodal gate and all; `a_refusal_that_changes_reason_is_not_a_repetition`.

### 3. `budget.low`, with the agent's name

- Emitted when an agent's headroom (`governance::headroom_cents`) falls under its learned cost of the next turn (`estimate_cents`) — before `check` would refuse. A notification (`notify::kind::BUDGET_LOW`, a person acts on it) and an audit event (a fact about the run).
- Once per agent per budget window, not once per beat; again after the cap is raised.
- **Tests first:** `budget_low_names_the_agent_before_the_wallet_says_no`, `budget_low_is_said_once_per_window`.

### 4. A stop is a wind-down, with a handoff

- **Advertised vs enforced.** The run's prompt states its advertised bounds — 85 % of the money bound passed to the provider, 90 % of `session_timeout_secs` — and asks for a handoff before reaching them: what was done, what is pending, what it proposes next. The enforced bounds stay where they are and fire only when the wind-down misbehaves.
- **Overmind writes the handoff when the run did not.** On `TimedOut` or a budget kill, Overmind composes its own record from durable state — the files the run changed in its worktree, the cost, the tail of its output — so no task that reached a bound is left with no account of itself. Stored as an artifact of the task (`kind = handoff`), visible on the task and passed to the next attempt's prompt.
- The Wadachi desk as handoff when memory is on (ADR-0050 §4) is **not** in this slice: Overmind's own record works with memory on and off (CLAUDE.md: memory is optional and first-class); the desk is a follow-up once the record's shape has met real runs.
- **Tests first** (stub agent, never the real CLI): `a_run_killed_at_its_deadline_leaves_a_handoff`, `the_next_attempt_reads_the_previous_handoff`, `the_prompt_states_the_advertised_bound_not_the_enforced_one`.

Each PR: the checks of CLAUDE.md, `/code-review medium`, `/security-review` for slices 3 (`governance.rs`) and 2 (audit chain), a CHANGELOG line, ROADMAP kept true.

## Done when

ADR-0050's acceptance for M36a, as tests and then live:

- a refusal repeated to its tolerance stops the task as `stalled` with the reason;
- a task failing the same way three times stops as `stalled`, not `todo` again;
- a run reaching an advertised bound leaves a handoff, never an empty task;
- `budget.low` names the agent before the wallet says no;
- **live on TravelAgency** (native, `~/overmind-casa`): a week with zero repeated refusals past the cap and no run ending without a handoff; then the ledger's numbers are read against the defaults (§10).

## Not in M36a

- The stagnation and cycling detectors — they need M35's check.
- Approval by reversibility, auto-acceptance, triggers — M36b.
- **Repetition of tool calls inside a run** (fingerprint of tool + arguments, response *inform → constrain → stop*). ADR-0050 §3 names it beside refused starts, but M36a's acceptance criterion does not, and it needs the run's tool stream parsed live — a slice of its own. Proposed as **slice 5 or the first of M36b**: the owner's call (below).

## Answered at approval

1. **Tool-call repetition** stays out of M36a; it is taken up again after slice 4, slices 1–4 not depending on it.
2. **Restarting a `stalled` task resets its count**: the person has looked.
