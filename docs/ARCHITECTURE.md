# Architecture

> **Status: living draft.** This document describes intent, not shipped code. Every significant choice here must have an ADR; if this file and an ADR disagree, the ADR wins until this file is updated.

## System overview

```
┌─────────────────────────────────────────────────────────────┐
│                        overmind-web                         │
│         React + TypeScript SPA (board, org chart,           │
│          diff review, audit browser, governance)            │
└───────────────▲─────────────────────────────────────────────┘
                │ HTTP + WebSocket (typed API)
┌───────────────┴─────────────────────────────────────────────┐
│                       overmind-server                       │
│                     Rust (axum + SQLite)                    │
│                                                             │
│  ┌────────────┐ ┌────────────┐ ┌───────────┐ ┌───────────┐  │
│  │ org model  │ │  tickets & │ │ scheduler │ │ governance│  │
│  │ roles/goals│ │ audit log  │ │(heartbeat)│ │ & budgets │  │
│  └────────────┘ └────────────┘ └───────────┘ └───────────┘  │
│  ┌────────────────────────┐  ┌────────────────────────────┐ │
│  │     agent runners      │  │       MCP layer            │ │
│  │ spawn CLIs in isolated │  │ client: tools + memory     │ │
│  │ git worktrees          │  │ server: expose Overmind    │ │
│  └───────────┬────────────┘  └──────────┬─────────────────┘ │
└──────────────│──────────────────────────│───────────────────┘
               ▼                          ▼
   Claude Code / Gemini CLI /     MemoryProvider (MCP) — OPTIONAL
   other agent CLIs, one          Wadachi first-party: managed
   worktree + branch each         per-org instance (ADR-0004)
```

## Components

### overmind-server (Rust)

- **Org model** — `Organization`, `Role`, `Agent`, `Mission`, `Goal`. Missions cascade into goals, goals into tickets. Reporting lines form a DAG (an agent has one manager; the root is the human owner).
- **Tickets & audit** — `Ticket` is the unit of assigned work; every state change, tool call and decision appends an `Event` to an **append-only audit log**. Events are never updated or deleted.
- **Scheduler** — heartbeat wake-ups per agent; an agent resumes its checked-out ticket with persisted context rather than restarting.
- **Governance & budgets** — per-agent monthly budgets. **Invariant: ticket checkout and budget reservation are a single atomic transaction** (this is the hard problem Paperclip solved; we adopt the same guarantee). Approval gates (hiring, spend over threshold, protected actions) are enforced server-side.
- **Agent runners** — spawn external agent CLIs as child processes, each in **its own git worktree on its own branch** (Vibe Kanban's model). Runner captures stdout/stderr, streams progress, enforces timeouts, and reports cost.
- **MCP layer** —
  - *client*: connects to configured MCP servers and exposes them to agents; the `MemoryProvider` is just a distinguished MCP connection.
  - *server*: exposes Overmind itself over MCP (create ticket, read board, query audit), so external agents can participate.

### overmind-web (React + TypeScript)

Kanban board (tickets by state), org chart view, diff review with inline comments, audit log browser, budget dashboards, approval inbox. Talks to the server over a typed API (OpenAPI-generated client); live updates over WebSocket.

### MemoryProvider contract

A thin trait over MCP with graceful degradation:

```
get_context(scope, task)   → relevant memories before an agent starts
recall(query)              → targeted lookup during work
store_memory(item)         → discoveries, patterns, bugfixes
store_decision(decision)   → choices with rationale
```

Rules: if no provider is configured, all calls are no-ops and Overmind is fully functional. Provider failures are logged, never fatal. Wadachi already implements a superset of this surface.

**Wadachi as first-party managed brain** ([ADR-0004](adr/0004-wadachi-first-party-managed-brain.md)): beyond the generic contract, Overmind can provision, launch and supervise a dedicated Wadachi instance per organization (brain dir at `<data-dir>/orgs/<org>/brain/` — never the user's personal brain). The server manages its lifecycle like any supervised child process; the web UI surfaces organizational memory first-class: memory browser, decisions linked to the tickets that produced them, provenance tracing ("which memory guided this action"). No Wadachi code is vendored into this repo — integration is MCP + process management only.

## Data & storage

SQLite via `sqlx` (single-file DB fits self-hosted; WAL mode; migrations checked in). Audit events additionally hash-chained (each event stores the hash of the previous one) so tampering is detectable.

## Security posture (v0 targets)

- Agent processes run with least privilege; workspace = their worktree, nothing else, enforced by OS-level sandboxing (`sandbox-exec` on macOS first, Linux later).
- Secrets live server-side and are injected into tools, never into agent prompt context.
- All approval gates enforced by the server; a prompt-injected agent cannot skip them.

## Tech stack

| Layer | Choice | Why (ADR) |
|---|---|---|
| Backend | Rust, axum, sqlx + SQLite, tokio | [ADR-0002](adr/0002-rust-backend-react-frontend.md) |
| Frontend | React, TypeScript, Vite | [ADR-0002](adr/0002-rust-backend-react-frontend.md) |
| Agent integration | External CLIs + MCP | [ADR-0003](adr/0003-memory-via-mcp-wadachi-optional.md) |
| Memory | `MemoryProvider` over MCP, optional; Wadachi first-party managed brain | [ADR-0003](adr/0003-memory-via-mcp-wadachi-optional.md), [ADR-0004](adr/0004-wadachi-first-party-managed-brain.md) |

## Reference codebases

- **Paperclip** (MIT) — feature design reference: org/budget/governance semantics, atomicity guarantees. Code may be ported with attribution.
- **Vibe Kanban** — pattern reference for Rust: worktree management, runner supervision, MCP server. Check its license before porting any code; ideas are free.
- **Aperant** (AGPL-3.0) — **ideas only, never code** (license incompatible with MIT).
