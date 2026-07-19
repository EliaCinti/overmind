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

## M1 — Domain core + audit log `todo` ← next
The data model and the accountability spine — before any agent runs.
- Entities: Organization, Role, Agent, Mission, Goal, Ticket, Event
- Append-only, hash-chained audit log; every ticket state change is an event
- SQLite migrations; typed HTTP API (CRUD + state transitions)
- **Accept:** tickets move through their lifecycle via API; audit log replays the full history; tampering with an event breaks the chain verification.

## M2 — First agent runs `todo`
One agent, one ticket, end to end.
- Runner spawns Claude Code CLI as child process in an isolated git worktree + branch
- Ticket checkout is atomic (two concurrent checkouts of the same ticket: exactly one wins)
- Output streamed and persisted; cost recorded on the ticket
- **Accept:** assign a real coding ticket, agent completes it in its worktree, diff is visible, every step is in the audit log.

## M3 — Parallelism + heartbeats `todo`
- N agents in parallel, one worktree each, no interference
- Heartbeat scheduler; agent resumes persisted ticket context across wake-ups
- Timeouts, crash recovery (a dead runner releases its ticket safely)
- **Accept:** 3+ agents work 3+ tickets concurrently across restarts of the server.

## M4 — Board UI `todo`
First real UI (React SPA).
- Kanban board with live updates (WebSocket), ticket detail with streamed output
- Diff review with inline comments
- **Accept:** Elia runs a full ticket lifecycle without touching the API directly.

## M5 — Organization `todo`
The company layer.
- Org chart (roles, reporting lines), missions → goals → tickets cascade
- Org chart UI
- **Accept:** a mission created at the top produces goal-linked tickets assigned per role.

## M6 — Budgets + governance `todo`
- Per-agent monthly budgets; **budget reservation atomic with ticket checkout**
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
- Overmind provisions, launches and supervises a dedicated Wadachi instance per organization (`<data-dir>/orgs/<org>/brain/`); never touches a personal brain
- Memory UI: browser of org memories, decisions linked to the tickets that produced them
- Depends on Wadachi supporting concurrent multi-agent access (tracked in the Wadachi repo)
- **Accept:** a fresh organization gets a working brain in one click; a decision stored by an agent is visible in the UI linked to its ticket; disabling the brain leaves the org fully functional.

## M9 — Overmind as MCP server `todo`
- Expose tickets/board/audit over MCP; external agents can file and read tickets
- **Accept:** a Claude Code session outside Overmind creates a ticket via MCP.

## M10 — Security hardening `todo`
- OS-level sandboxing of runners (macOS `sandbox-exec` first); secrets isolation; threat-model doc; prompt-injection review of every gate
- **Accept:** a deliberately malicious ticket ("read ~/.ssh, push to main, exceed budget") fails at every layer.

## Later / icebox
Linux/Windows support · multi-user · plugin system · agent marketplace-style role templates · non-coding agent workflows (research, content) · public release polish
