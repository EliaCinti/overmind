# ADR-0008: Execution sessions, atomic checkout, worktree isolation

- **Date:** 2026-07-19
- **Status:** accepted

## Context

M2 needs an agent to actually run a task: checkout without double-work, an isolated place to work, captured output, recorded cost. Paperclip models execution as `agent_task_sessions` (+ `cost_events`, `project_workspaces`); Vibe Kanban isolates every run in its own git worktree.

## Decisions

1. **Paperclip's tables, verbatim shape.** `project_workspaces` (a project's git repo: `cwd`, `default_ref`, `is_primary`), `agent_task_sessions` (`adapter_type`, status `queued/running/completed/failed`, `last_error`), `cost_events` (provider, model, token counts, `cost_cents`).
2. **Atomic checkout via conditional UPDATE.** `UPDATE tasks SET status='in_progress', assignee=? WHERE id=? AND status='todo'` — of N concurrent callers exactly one affects a row; losers get 409. Checkout, session insert and audit events commit in one transaction. No locks, no queue: SQLite's serialized writes make this race-free.
3. **Worktree per session** (Vibe Kanban's model): `git worktree add <data-dir>/worktrees/<session-id> -b overmind/task-<id8>-<sess8> [default_ref]`, run from the workspace repo. `base_sha` recorded at start; the session diff is `git add --intent-to-add --all && git diff <base_sha>` (intent-to-add so files the agent *created* appear too — plain `git diff` ignores untracked files).
4. **Adapter command is configuration.** Default drives the Claude Code CLI headless (`claude -p "$OVERMIND_TASK_PROMPT" --output-format json`); `OVERMIND_AGENT_CMD` overrides it (tests use a stub shell script). The command runs via `sh -c` with the task context in env vars, cwd = the worktree.
5. **Cost from the final JSON line.** The Claude Code CLI's JSON result carries `total_cost_usd` + `usage`; the runner parses the last JSON object in the output and writes a `cost_events` row. Missing cost is not an error.
6. **Failure semantics: `blocked`.** Non-zero exit or infrastructure error → session `failed` (with `last_error`), task → `blocked` (needs human attention; `blocked → in_progress` allows retry). Success → task `in_review`.

## Alternatives considered

- **`SELECT ... FOR UPDATE` / advisory locks** — not available/necessary in SQLite; the conditional UPDATE is simpler and provably atomic. Rejected.
- **A queue table with a claiming worker** — right shape for M3 heartbeats, premature for M2's on-demand start. Deferred.
- **Naming the table `runs`** — Paperclip canon is `agent_task_sessions`; sessions become resumable in M3, so the name will fit even better. Rejected `runs`.

## Consequences

- Worktree cleanup (`git worktree remove`) is **not** done yet — sessions keep their worktree for diff/review. Cleanup policy lands with M3 (crash recovery) / M4 (review UI actions).
- No timeout enforcement yet — an agent that hangs holds its task in `in_progress`. Scheduled for M3, as per roadmap.
- The prompt currently instructs agents to leave changes uncommitted; merge/commit flows belong to the review milestone.

## Addendum — which repository a task runs in (2026-08-08)

Decision 3 assumes a task knows its repository: `task → goal → project →
primary workspace`. Nothing said what happens when the first link is missing,
and it was missing more often than anyone thought. `POST /api/companies/{id}/tasks`
has always accepted a `code` task with no `goal_id`, and `ceo.rs` bound that
column to `NULL` unconditionally — so **every `code` task an agent opened from
a chat or a meeting was born unable to run**. The frontend had the answer all
along (`web/src/lib/repo.ts` creates "the default goal that `code` tasks attach
to"; `CreateTaskDialog` passes it and refuses a code task without a repo), and
the server, which is where agents live, had never been told.

Found in the live smoke run, by clicking Start on a task the CEO had just
opened. Two families of tests were green over the seam: some watched an agent
open a task and stopped at "the row exists", others ran a `code` task the test
had created by hand, with a goal. Neither crossed.

**The rule, applied at both ends: never guess which repository, filing within
one is fine.**

- **When the task is opened**, the server resolves the company's default goal
  the way the frontend does — but only when the company has exactly one
  repository-backed project. A second one makes the answer `None` and the task
  is left visibly unattached, because which codebase an agent works in is a
  decision with consequences and a wrong guess is invisible in the result.
  Choosing among several goals of the *same* project only decides where the
  task is filed, so the oldest wins and a human can move it.
- **When the task is started**, an unattached `code` task falls back to the
  company's sole primary workspace. Orphans predate this change and will keep
  arriving through the API, so a creation-time fix alone would leave them dead
  forever. With more than one repository the start is refused and says why.

**Consequence:** the fallback means `goal_id` is not what *finds* the
repository at start, only what records the intended one. That is a small
redundancy, kept deliberately: the alternative is a task that can never run and
no way to say so.

Held by `execution.rs` — `a_code_task_an_agent_opens_can_actually_be_started`,
`an_orphaned_code_task_runs_when_there_is_only_one_repository`,
`overmind_will_not_guess_which_repository_an_agent_works_in`.
