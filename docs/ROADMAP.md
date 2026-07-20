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

## M5 — Company `todo`
The company layer.
- Org chart (roles, reporting lines), projects → goals → tasks cascade
- Org chart UI
- Guided agent hiring ([UX.md](UX.md) reference flow): archetype gallery → structured tuning → expert mode, with live "what this agent will do" preview
- **Accept:** a project created at the top produces goal-linked tasks assigned per role; a non-expert hires a working Security Engineer agent in under a minute without typing free text.

## M6 — Budgets + governance `todo`
- Per-agent monthly budgets; **budget reservation atomic with task checkout**
- Approval gates (spend threshold, protected actions), approval inbox in UI
- Pause / terminate agents; config changes revisioned with rollback
- **Accept:** an agent that would exceed budget is stopped server-side; a gated action blocks until human approval.

## M7 — Memory contract `todo`
The differentiator, part 1: the generic `MemoryProvider` interface (ADR-0003).
- MCP client layer; `MemoryProvider` contract implemented against an externally-run Wadachi
- Agents call get_context on wake, store memories/decisions on completion
- Graceful degradation verified: everything works with no provider configured
- **Accept:** demonstrate an agent avoiding a past mistake because the org remembered it; then unplug Wadachi and show identical (memoryless) behavior.

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
