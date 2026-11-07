# A run ends on the task it started

*Plan, 1 Nov 2026. Closes the debt "A run that ends overwrites a task a person
cancelled while it ran" (NEXT.md, *Debts from the attempts cap*), which NEXT.md
puts in *Now*, during M36a's week live.*

## The problem

`runner::finalize` moves the task with three `UPDATE tasks … WHERE id = ?`
(`stalled`, `todo` on release, `task_to` = `in_review` | `blocked` | `todo`),
and each writes an audit row that says `from: in_progress`. Nothing checks that
the task is still the one the run checked out.

It is wider than *cancelled*. `can_transition` lets a person move a task out of
`in_progress` by hand to `todo`, `blocked` or `cancelled`, and the run is not
stopped. In every one of the three, the run's end overwrites what the person
decided and the chain records a move from a state the task was not in. The
worst is `todo`: the scheduler can check the task out again, a **second**
session starts, and when the **first** run ends it sees `in_progress` (the
second run's) and writes its own outcome over it. A guard on `status =
'in_progress'` alone does not catch that one.

Two more places assume the same thing:

- `release_dependents` runs on every completed session: a run that completes
  on a task the person cancelled releases its dependents with its deliverables
  as inputs.
- `recover_orphans` resumes every `queued`/`running` session after a restart,
  whatever its task now says: a run of a cancelled task comes back to life.

## The approach

**The run applies its outcome only if the task is still its own**: still
`in_progress`, and no session of that task was created after this one.

1. One guard, written once, used by the three `UPDATE`s in `finalize`:
   `AND status = 'in_progress' AND NOT EXISTS (SELECT 1 FROM
   agent_task_sessions s WHERE s.task_id = tasks.id AND s.created_at >
   (SELECT created_at FROM agent_task_sessions WHERE id = ?))`. The audit
   `task.transitioned` / `task.released` / `task.stalled` rows are written only
   when `rows_affected() == 1`.
2. When the guard does not match, the session is still recorded in full
   (status, output, cost, artifacts, branch returned): what the run did and
   cost is true regardless of what happened to the task. `session.finished`
   carries `"applied": false` and the task's status at that moment, so the
   chain says why the task did not move.
3. `release_dependents` only when the outcome was applied. The memory written
   at the end of a completed run stays as is: what the run learned is still
   learned.
4. `recover_orphans` does not resume a session whose task is no longer
   `in_progress`: it closes it as `failed` with `last_error` saying the task
   was moved while the server was down (through the i18n helper, like every
   server sentence meant for a person), and goes through the same unapplied
   path.

**Out of scope, written down.** Stopping the process when a person moves the
task: today a run of a cancelled task keeps spending until its own end. It is
the right next step, but it touches the process handles in `running` and the
cage's process group, so it wants a change of its own. It goes into NEXT.md
under the same debt, which this change narrows rather than closes.

## Tests first (in `tests/scheduler.rs`)

A gated stub: it waits for a file the test creates, then exits with whatever
the outcome needs. The test moves the task through the API while the run waits,
then opens the gate and waits for the session to end.

- `a_task_cancelled_while_its_run_works_stays_cancelled`: the run succeeds
  (`in_review`), the task stays `cancelled`, and no audit row says
  `from: in_progress` after the cancellation.
- `a_task_blocked_by_hand_while_its_run_works_stays_blocked`: the run fails
  (`blocked`).
- `a_run_that_releases_does_not_unassign_a_cancelled_task`: the release path.
- `a_run_at_the_cap_does_not_stall_a_cancelled_task`: the stalled path
  (`attempts_cap = 1`).
- `a_run_that_ends_after_the_task_was_restarted_leaves_the_new_run_alone`: by
  hand to `todo`, a second session checks it out; the first ends; the task is
  still `in_progress` on the second session, and the second's outcome applies.
- `a_completed_run_of_a_cancelled_task_releases_no_dependents`.
- `a_session_of_a_cancelled_task_is_not_resumed_after_a_restart`.
- The existing runs where nothing moved the task stay green: they are the guard
  that the normal path still applies.

Each must fail for the expected reason before the fix.

## Done when

- The seven tests above fail before and pass after; the 38 suites stay green.
- The CLAUDE.md checks are green (fmt, clippy `-D warnings`, test, web).
- `/code-review medium` is done and every finding is fixed or written down.
  `/security-review` is not needed: no change to the cage, the door or the
  credentials.
- CHANGELOG has a `Fixed` entry. NEXT.md: the debt is narrowed to "stopping the
  process" in a separate commit, since NEXT.md also has uncommitted account
  changes that do not belong to this PR.
- A PR from `main` on `fix/a-run-ends-on-its-own-task`, merged by Elia, then
  Casa rebuilt and restarted (DB backup first). That restart sits inside M36a's
  week live and changes no M36a code.
