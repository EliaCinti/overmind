# The task knows its run

*Plan, 1 Nov 2026. Supersedes the predicate of
[a run ends on the task it started](2026-11-01-a-run-ends-on-the-task-it-started.md)
after `/code-review medium` on its branch; that plan stays as the record of
the first attempt.*

## Why the first predicate is wrong

"Still `in_progress` and no session created after this one" infers ownership
from the clock and from session rows. It fails three ways:

- `Blocked → InProgress` and `InReview → InProgress` are hand moves that create
  no session. Block a task mid-run, put it back in progress, and the old run
  finds it `in_progress` with no later session, so it applies over the
  person's move.
- A strict `>` on RFC3339 wall-clock strings fails open when the clock steps
  back or two checkouts share an instant.
- `resume_session` does not use it. After a restart, a stale run of a task
  that was requeued and started again is resumed and spends a second time.

## The approach

**The task records which run holds it.** Migration 0035 adds
`tasks.run_session_id`.

- The checkout sets it, in the same `UPDATE … AND status = 'todo'`.
- `transition_task` clears it on every hand move.
- `finalize` clears it when it applies.
- "The run still owns the task" is one function in `runner.rs`:
  `status = 'in_progress' AND run_session_id = ?`, one query. `finalize` uses
  it to decide `applied`; `resume_session` uses it to decide whether to resume.
  A session it does not own is closed as failed, and the closing goes through
  the same unapplied path.
- Backfill: an `in_progress` task gets its newest `queued`/`running` session,
  so runs in flight across the upgrade keep their task.

**What an unapplied run still does, and what it no longer does:**

- Still done: the session is recorded in full (output, cost, artifacts, the
  branch returned), and `session.finished` carries `applied: false` and the
  status it found.
- No longer done:
  - it releases no dependents;
  - it raises no meeting request;
  - it writes no "task completed" memory. This reverses the first plan: a
    memory tied to a task the person threw away reports as done work that is
    not;
  - its `failure_cause` is stored as NULL, so it does not count toward the
    attempts cap of the runs that own the task.

**Found by the review, pre-existing, fixed here:** the dependents release and
the memory read `f.session_status`, but an empty-handed run is downgraded to
`failed` after it. Both now read the effective `session_status`.

## Tests first

On top of the seven already on the branch:

- `a_task_blocked_and_resumed_by_hand_is_not_the_old_runs_anymore`
- `a_stale_run_is_not_resumed_when_its_task_was_started_again`
- `an_unapplied_failure_does_not_count_toward_the_cap`
- `an_unapplied_run_raises_no_meeting_and_writes_no_memory`
- `an_empty_handed_run_releases_no_dependents`
- `a_run_in_flight_across_the_migration_keeps_its_task`, a backfill test that
  inserts a session plus an `in_progress` task with NULL `run_session_id`,
  then runs the migration's backfill statement.

## Done when

- The new tests fail before and pass after, together with the seven already
  on the branch.
- The CLAUDE.md checks are green.
- `/code-review medium` runs again, and every finding is fixed or written down.
- `/security-review` has run: the audit chain is touched.
- The PR body says what changed from the first plan and why.
