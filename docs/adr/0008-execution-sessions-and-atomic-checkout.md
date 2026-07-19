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
