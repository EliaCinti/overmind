# ADR-0017: Knowledge execution and task artifacts

- **Date:** 2026-07-27
- **Status:** accepted

## Context

[ADR-0016](0016-general-purpose-conversational-company.md) commits Overmind to general-purpose work, and names **deliverable-agnostic execution** as the foundation (M11): agents must be able to produce **documents/research/decisions**, not only code diffs. Today ([ADR-0008](0008-execution-sessions-and-atomic-checkout.md)) every run is a git **worktree** whose deliverable is a **diff** — there is nowhere to put a non-code result, and the drawer only knows how to show a diff.

We need the smallest change that lets an agent produce a document, without disturbing the code path, and while keeping the lifecycle, governance, audit and memory that already work.

## Decision

Introduce an **execution kind** per task and an **artifact** as the general deliverable. Sessions ([ADR-0008](0008-execution-sessions-and-atomic-checkout.md)) stay the unit of "an agent run on a task"; only *what a run produces and where it is stored* changes.

1. **`execution_kind` on the task.** New column `tasks.execution_kind TEXT NOT NULL DEFAULT 'code'` — `code` | `knowledge`. Default `code` keeps every existing task and test behaving exactly as before (backward-compatible migration). A project may carry a `default_execution_kind` so a "home cinema" project defaults its tasks to `knowledge`; the CEO/creator can still set it per task.

2. **Knowledge runs have no worktree.** For `knowledge`, the runner skips `git worktree add` / `base_sha` / diff entirely. It creates a plain **session scratch directory** (`<data-dir>/sessions/<session-id>/`) as the agent's `cwd`, runs the same configurable adapter (`OVERMIND_AGENT_CMD`) with the task prompt + memory context + (later, M12) attachments, and instructs the agent to write its deliverable(s) as files into that dir and/or return them in its final JSON.

3. **`task_artifacts` — the general deliverable.** New table: `id`, `task_id`, `session_id`, `kind` (`document` | `table` | `research` | `decision` | `link`), `title`, `mime`, `content` (inline text/markdown) **or** `file_path` (for binary/large), `created_at`. A session registers one or more artifacts; the runner captures them from (a) new files written into the scratch dir and (b) the adapter's final JSON. The plain-text run output is also stored as a `document` artifact (the "summary"), so there is always at least one.

4. **The drawer shows artifacts for `knowledge`, diff for `code`.** The task-detail drawer branches on `execution_kind`: `GET /sessions/{id}/diff` remains for `code`; a new `GET /tasks/{id}/artifacts` (and `GET /artifacts/{id}`) serves `knowledge`. Same live-update channel.

5. **Everything else is unchanged and inherited.** Same task lifecycle + transition table ([ADR-0006](0006-audit-log-and-task-lifecycle.md)); atomic budget checkout and approval gates ([ADR-0012](0012-budgets-and-governance.md)) apply identically (a knowledge run reserves/spends budget like any run); memory `get_context` on start / `store_memory` on finish ([ADR-0013](0013-memory-over-mcp.md)); audit gains one event kind, `artifact.created`. Success → `in_review` (a human reads the document), failure → `blocked`, exactly as today.

## Alternatives considered

- **A separate "document task" entity parallel to Task.** Doubles the lifecycle, board, governance, and audit surfaces. Rejected — the task *is* the right unit; only its deliverable varies. One `execution_kind` column beats a parallel hierarchy.
- **Keep the git worktree, commit the document into it.** Forces every knowledge project to be a git repo and re-uses diffs for prose — the wrong mental model, and it drags git semantics into non-code work. Rejected; `knowledge` runs are git-free.
- **Store artifacts only as loose files on disk (no table).** Loses queryability, the audit link, and the board/API surface. Rejected — artifacts are first-class rows; large/binary payloads go to `file_path`, text stays inline.
- **Infer the deliverable purely from the adapter's stdout.** Brittle. Rejected in favour of an explicit contract: agent writes files to the scratch dir and/or returns artifacts in the final JSON; the runner captures both.

## Consequences

- **Migration is additive and safe:** one nullable-with-default column + one new table; no existing row or test changes meaning. The `code` path is untouched.
- **First usable slice (M11 accept):** create a `knowledge` task from the board → the agent produces a document artifact → visible in the drawer → audited, chain intact; a `code` task still diffs as before. Exercisable from the board **without** the chat (M12 drives it later).
- **Scratch-dir cleanup** mirrors the open worktree-cleanup question (ADR-0008 consequences): sessions keep their scratch dir for artifact review; a retention policy lands with the review/cleanup work.
- **Security (M10) now spans two runtimes:** the knowledge scratch dir is not a git worktree but still runs an adapter with full tool access — sandboxing must cover it too. Noted for M10.
- **Adapter guidance:** the default prompt for `knowledge` tasks tells the agent to write its deliverable as a file (e.g. `ARTIFACT.md`) and summarize; a real capability set (web research, spreadsheet output) is characterization work (M14), not this ADR.
- **"Where this lives" (owner map):** `execution_kind` + artifacts touch `domain.rs` (types + `event_kind`), a migration under `migrations/`, `runner.rs` (branch on kind: worktree vs scratch dir, capture artifacts), `api.rs` (`/tasks/{id}/artifacts`), `db.rs` (queries), and the drawer in the React app. This is the list to change when we build M11.
