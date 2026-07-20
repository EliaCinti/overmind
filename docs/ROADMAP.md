# Roadmap

> Single source of truth for "what's next". The MVP vision is big; the working unit is the **milestone**, and a milestone is done only when its acceptance criteria pass. We do not start milestone N+1 with N half-finished.
>
> Status legend: `todo` · `in-progress` · `done`

## M0 — Foundations `done`
Repo, docs, decisions.
- [x] Repo + git init
- [x] VISION / ARCHITECTURE / ROADMAP / ADRs 0001–0004
- [x] Rust workspace skeleton (`overmind-server`), CI workflow (fmt, clippy, test), `cargo run` serves `/health` — verified locally (fmt+clippy+test green, endpoint answers)
- [x] First commit; public GitHub repo ([EliaCinti/overmind](https://github.com/EliaCinti/overmind)) + first green CI run (2026-07-19)
- **Accept:** a fresh clone builds, tests pass in CI, docs explain the project to a stranger. ✓

## M1 — Domain core + audit log `done`
The data model and the accountability spine — before any agent runs.
- [x] Entities: Company, Role, Agent, Project, Goal, Task, Event, Archetype (Role has schema + FK only; its API arrives with the org chart in M5)
- [x] Agent characterization structured per [ADR-0005](adr/0005-structured-agent-characterization.md): archetype + typed traits + additive `custom_brief`; built-in catalog seeded (6 archetypes)
- [x] Append-only, hash-chained audit log ([ADR-0006](adr/0006-audit-log-and-task-lifecycle.md)); every mutation appends an event atomically
- [x] SQLite migrations (sqlx); HTTP API with typed, validated requests (CRUD + state transitions)
- **Accept:** tasks move through their lifecycle via API ✓; audit log replays the full history ✓; tampering with an event breaks the chain verification ✓ (all covered by integration tests, 2026-07-19).

## M2 — First agent runs `done`
One agent, one task, end to end.
- [x] Runner spawns the agent CLI (Claude Code by default, adapter command configurable via `OVERMIND_AGENT_CMD`) in an isolated git worktree + branch ([ADR-0008](adr/0008-execution-sessions-and-atomic-checkout.md))
- [x] Task checkout is atomic — integration test proves two concurrent checkouts yield exactly one 202 and one 409
- [x] Output captured and persisted on the session; cost recorded as Paperclip-style `cost_events` (parsed from the CLI's JSON result)
- [x] `project_workspaces` + `agent_task_sessions` per Paperclip schema; failed sessions block the task with `last_error`
- **Accept:** agent completes a task in its worktree ✓, diff visible via `GET /sessions/{id}/diff` ✓, every step audited with the chain still verifying ✓ (integration tests, 2026-07-19; verified with the stub adapter — a paid smoke run with the real Claude Code CLI is a manual follow-up).

## M3 — Parallelism + heartbeats `done`
- [x] N agents in parallel, one worktree each, no interference (test: 3 agents × 3 tasks concurrently, distinct worktrees)
- [x] Heartbeat scheduler ([ADR-0009](adr/0009-heartbeat-scheduler-and-recovery.md)); orphaned sessions resumed after a simulated server restart, `resumed_count` bumped
- [x] Timeouts kill the session and **release** the task to `todo`; agent failures **block** it — distinct recovery semantics
- [x] `agent_wakeup_requests` (Paperclip-shaped); wakeups auto-start work only for `act_within_budget` agents (autonomy enforced server-side per ADR-0005)
- **Accept:** 3+ agents work 3+ tasks concurrently ✓; orphaned session recovered across a restart ✓; audit chain still verifies throughout ✓ (integration tests, 2026-07-20, stub adapter).

**M3 bug caught by the parallelism test:** short UUIDv7 prefixes collided across same-millisecond sessions → duplicate branch names. Branch now uses the full session id.

## M4 — Board UI `done`
First real UI (React SPA), best-in-class graphical stack ([ADR-0010](adr/0010-frontend-stack-and-live-updates.md)).
- [x] Stack: Vite + React + TypeScript + Tailwind v4 + Radix + Motion + Lucide, self-hosted Inter/JetBrains fonts, OKLCH light/dark design tokens
- [x] Kanban board (tasks by status) with live updates over WebSocket (coarse "company changed" → refetch); task detail drawer with session output + git diff
- [x] Guided agent hiring with progressive disclosure (archetype gallery → structured tune → expert mode) and a live "what this agent will do" preview — the UX.md signature flow, shipped early here
- [x] First-run onboarding (name company → connect git repo), audit-chain trust indicator, theme toggle
- [x] Server serves the built SPA at root with history fallback; API nested under `/api`; `/ws` live channel
- **Accept:** full task lifecycle driven from the UI ✓ — verified end-to-end (server serves SPA + API, company→workspace→agent→task→start→session completed→diff→audit valid, stub adapter, 2026-07-20). Live diff review is read-only; inline diff *comments* deferred to review-milestone work.

## M5 — Company `done`
The company layer — the people structure.
- [x] Reporting hierarchy on agents (`reports_to` → agent, `title`), Paperclip-aligned ([ADR-0011](adr/0011-org-hierarchy-on-agents.md)); the M1 `roles` table dropped
- [x] Reporting DAG enforced server-side: no self-reporting, cycle-creating reassignment rejected (400); `POST /agents/{id}/reassign`
- [x] Org chart UI: reporting tree rooted at the human owner, inline manager/title editing, "hire a report" under any node; hire dialog gains title + "reports to"
- [x] Board ↔ Org view switch
- Projects → goals → tasks cascade already exists (M1/M2); guided hiring shipped in M4
- **Accept:** agents assembled into a reporting hierarchy and re-organized from the UI ✓; the DAG invariant holds under a cycle attempt ✓; a non-expert hires a working Security Engineer in one click without free text ✓ (org integration tests + E2E, 2026-07-20). Auto-decomposition of a project into per-role tasks is agent behavior, deferred (see ADR-0011).

## M6 — Budgets + governance `done`
What makes Overmind safe to leave running ([ADR-0012](adr/0012-budgets-and-governance.md)).
- [x] Per-agent monthly budgets (the `monthly_budget_cents` trait is the cap); **enforcement atomic with task checkout** — spent + reserved + estimate must fit, else the start is refused (402) and a `budget_incidents` row is written. Per-session reservation released on finish.
- [x] Approval gate: a `requires_approval` agent files a pending approval on start and launches nothing; approving it runs the start (bypassing the gate once), rejecting leaves the task. Approvals inbox in the UI.
- [x] Agent lifecycle: pause / resume / terminate (paused/terminated can't start; terminate permanent)
- [x] Config revisions on every hire/reassign; `POST /agents/{id}/rollback` restores a past revision and appends a new forward-only `rollback` revision
- **Accept:** an over-budget start is stopped server-side (402, task never checked out) ✓; a gated start blocks until a human approves, then runs ✓; audit chain verifies throughout ✓ (4 governance integration tests + E2E, 2026-07-20). Config-revision UI and warn-threshold (80%) incidents deferred (see ADR-0012).

## M7 — Memory contract `done`
The differentiator, part 1: memory over MCP ([ADR-0013](adr/0013-memory-over-mcp.md)).
- [x] MCP client (JSON-RPC over stdio, per-call sessions); memory server is configured via `OVERMIND_MEMORY_CMD` — Overmind never imports Wadachi
- [x] The loop: `get_context` on start injected into the agent's prompt (+ `OVERMIND_MEMORY_CONTEXT`), `store_memory` on successful finish
- [x] Best-effort everywhere: no server → no-ops; broken/slow server → logged and swallowed, never fatal (timeout 30s)
- [x] `GET /memory/status` + a UI memory indicator
- **Accept:** verified against **real Wadachi** (throwaway brain) — task 1's completion was stored, task 2's agent received it via real `get_context` (avoiding the "past mistake") ✓; with no provider and with a deliberately broken provider, tasks complete identically ✓ (3 memory integration tests + real-Wadachi E2E, 2026-07-20).

## M8 — Managed brain + memory UI `todo`
The differentiator, part 2: Wadachi as first-party brain (ADR-0004).
- Overmind provisions, launches and supervises a dedicated Wadachi instance per company (`<data-dir>/companies/<company>/brain/`); never touches a personal brain
- Memory UI: browser of org memories, decisions linked to the tasks that produced them
- Depends on Wadachi supporting concurrent multi-agent access (tracked in the Wadachi repo)
- **Accept:** a fresh company gets a working brain in one click; a decision stored by an agent is visible in the UI linked to its task; disabling the brain leaves the org fully functional.

## M9 — Overmind as MCP server `todo`
- Expose tasks/board/audit over MCP; external agents can file and read tasks
- **Accept:** a Claude Code session outside Overmind creates a task via MCP.

## M10 — Security hardening `todo`
- OS-level sandboxing of runners (macOS `sandbox-exec` first); secrets isolation; threat-model doc; prompt-injection review of every gate
- **Accept:** a deliberately malicious task ("read ~/.ssh, push to main, exceed budget") fails at every layer.

## Later / icebox
Linux/Windows support · multi-user · plugin system · agent marketplace-style role templates · non-coding agent workflows (research, content) · public release polish
