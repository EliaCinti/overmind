<p align="center">
  <a href="#quickstart"><strong>Quickstart</strong></a> &middot;
  <a href="docs/VISION.md"><strong>Vision</strong></a> &middot;
  <a href="docs/ARCHITECTURE.md"><strong>Architecture</strong></a> &middot;
  <a href="docs/ROADMAP.md"><strong>Roadmap</strong></a> &middot;
  <a href="https://github.com/EliaCinti/overmind"><strong>GitHub</strong></a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/server-Rust-orange" alt="Rust" />
  <img src="https://img.shields.io/badge/ui-React%20%2B%20TS-06b6d4" alt="React + TypeScript" />
  <img src="https://img.shields.io/badge/status-pre--alpha-yellow" alt="Status" />
</p>

<br/>

# Overmind — the mind that runs your agent company.

Open-source orchestration for teams of AI agents — **with a memory.**

**If an agent is an _employee_, Overmind is the _company_ — and it remembers.**

Overmind is a Rust server and a React UI that organizes AI agents into a company: org chart, budgets, governance, isolated git worktrees, and a tamper-evident audit trail. Bring your own agents, assign work, and watch it happen from one board.

What makes it different: Overmind is **memory-native**. Through a pluggable interface (MCP), the whole organization shares a persistent brain — decisions with their _why_, patterns discovered, mistakes already made — that survives across sessions. The reference brain is [Wadachi](https://github.com/EliaCinti/wadachi); the interface stays open, and Overmind runs perfectly without one.

**Manage the work, not the terminals.**

|        | Step               | Example                                                                 |
| ------ | ------------------ | ----------------------------------------------------------------------- |
| **01** | Start a company    | Name it, point it at a git repo. One screen, you're running.            |
| **02** | Hire an agent      | Pick an archetype (_Security Engineer_, _Backend Dev_…), one click.     |
| **03** | Create a task      | Describe the work. An agent picks it up, in its own isolated worktree.  |
| **04** | Review & remember  | Read the diff, approve. The org stores what it learned for next time.   |

<br/>

## Overmind is right for you if

- ✅ You run **several coding agents at once** and lose track of who's doing what
- ✅ You want agents that **learn from the org's past** instead of starting cold every session
- ✅ You want every agent to work in an **isolated git worktree**, with the diff in front of you before anything merges
- ✅ You want **budgets enforced server-side** — an agent over its cap is stopped, not trusted to behave
- ✅ You want **governance**: approve a start, pause or terminate an agent, roll back a config change
- ✅ You want a **tamper-evident** record of everything that happened — provably, not just a log file
- ✅ You want it **self-hosted**, no account, your data on your machine

<br/>

## Features

<table>
<tr>
<td align="center" width="33%">
<h3>🧠 Organizational Memory</h3>
The whole org shares a persistent brain over MCP (Wadachi). Agents load context before working and record what they learned. <em>Nobody else has this.</em>
</td>
<td align="center" width="33%">
<h3>🔌 Bring Your Own Agent</h3>
Any agent CLI, one org chart, via a configurable adapter command. Claude Code by default; point it at whatever you run.
</td>
<td align="center" width="33%">
<h3>🌳 Isolated Worktrees</h3>
Every run gets its own git worktree and branch. Agents never step on each other; you review the diff before it lands.
</td>
</tr>
<tr>
<td align="center">
<h3>🔒 Hash-Chained Audit</h3>
Every action is an append-only, SHA-256-chained event. Tamper with history and verification pinpoints the exact broken event.
</td>
<td align="center">
<h3>💰 Atomic Budgets</h3>
Per-agent monthly caps, enforced <em>inside</em> the task-checkout transaction. Over budget → stopped server-side. No runaway spend.
</td>
<td align="center">
<h3>🛡️ Governance</h3>
Approval gates that block until you sign off. Pause / resume / terminate agents. Config revisions with forward-only rollback.
</td>
</tr>
<tr>
<td align="center">
<h3>📊 Org Chart</h3>
Agents have titles and reporting lines; the reporting DAG is enforced server-side (no cycles). Hire a report under any node.
</td>
<td align="center">
<h3>💓 Heartbeats & Recovery</h3>
A scheduler wakes agents, resumes interrupted sessions after a restart, and releases timed-out work safely.
</td>
<td align="center">
<h3>🎨 Guided Hiring</h3>
Progressive disclosure: pick an archetype, tune with clicks, drop into expert mode only if you need it. Live "what this agent will do" preview.
</td>
</tr>
</table>

<br/>

## Problems Overmind solves

| Without Overmind                                                                                   | With Overmind                                                                                        |
| -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| ❌ Ten agent terminals open; on reboot you lose which did what.                                    | ✅ Work is task-based, sessions persist and resume across restarts, every step is on the board.      |
| ❌ Every session your agent starts cold — you re-paste the same context and it repeats old mistakes. | ✅ The org remembers: agents load past decisions and patterns before they start.                     |
| ❌ Agents edit the same tree and clobber each other.                                                | ✅ One isolated git worktree per run; concurrent agents never interfere; you review each diff.        |
| ❌ A runaway loop burns hundreds of dollars before you notice.                                      | ✅ Budgets are enforced atomically at checkout; an over-budget agent is stopped, incident recorded.  |
| ❌ "Did the agent really do what it claimed?" — you can't prove it.                                 | ✅ An append-only hash chain: history is tamper-evident and verifiable end to end.                    |

<br/>

## Why Overmind is special

|                                    |                                                                                                                    |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| **Memory-native.**                 | A pluggable `MemoryProvider` over MCP — the org accumulates judgment across sessions. Optional, never a lock-in.    |
| **Atomic execution.**              | Task checkout and budget reservation commit in a single transaction — no double-work, no overrun.                  |
| **Tamper-evident by construction.**| The audit log is append-only (SQLite triggers) _and_ SHA-256 hash-chained; `GET /audit/verify` proves it.          |
| **Enforced, not suggested.**       | Archetype choices compile to server-enforced config (permissions, budget, gates) — a prompt can't override limits. |
| **Correctness-first stack.**       | Rust server (axum + SQLite), typed React UI — the concurrency-critical parts get compile-time guarantees.          |
| **Graceful degradation.**          | No memory server? Broken one? Tasks run identically. Memory failures are logged, never fatal.                      |

<br/>

## What's under the hood

```
┌──────────────────────────────────────────────────────────────┐
│                       OVERMIND SERVER (Rust)                  │
│                                                              │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐  │
│  │ Company & │  │  Tasks &  │  │ Heartbeat │  │Governance │  │
│  │ Org Chart │  │   Board   │  │ Scheduler │  │& Approvals│  │
│  └───────────┘  └───────────┘  └───────────┘  └───────────┘  │
│                                                              │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐  │
│  │  Agent    │  │ Worktrees │  │ Budgets & │  │  Audit    │  │
│  │  Runners  │  │ & Sessions│  │   Costs   │  │(hash-chain)│ │
│  └───────────┘  └───────────┘  └───────────┘  └───────────┘  │
│                                                              │
│  ┌───────────────────────────┐  ┌─────────────────────────┐  │
│  │  MCP client (memory)      │  │  MCP server (expose)    │  │
│  └───────────────────────────┘  └─────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
      ▲                ▲                          ▼
┌─────┴─────┐   ┌──────┴──────┐        ┌──────────┴──────────┐
│Claude Code│   │ any agent   │        │ Wadachi (memory)    │
│ / adapters│   │ CLI adapter │        │ — first-party,      │
└───────────┘   └─────────────┘        │   optional (MCP)    │
                                       └─────────────────────┘
```

**Company & Org Chart** — Companies scope everything. Agents have archetypes, titles, and reporting lines; the reporting DAG is enforced server-side. Projects cascade into goals and tasks.

**Tasks & Board** — A kanban board of tasks (backlog → todo → in_progress → in_review → blocked → done), with a live-updating UI over WebSocket and diff review.

**Agent Runners & Sessions** — Each task runs an agent CLI in its own git worktree/branch; output and cost are captured; sessions resume across restarts.

**Budgets & Governance** — Per-agent monthly budgets enforced atomically at checkout; approval gates; pause/resume/terminate; config revisions with rollback.

**Audit (hash-chained)** — Every mutation appends an immutable, SHA-256-chained event, committed in the same transaction as the change it records.

**Memory (MCP)** — Overmind speaks MCP to a memory server (Wadachi is first-party) so the org remembers — and exposes itself over MCP so external agents can file and read tasks.

<br/>

## Quickstart

### Docker (recommended)

```sh
docker compose up --build          # → http://localhost:7070
```

Persists the DB, worktrees and brains on a named volume. Mount your repos and set `OVERMIND_AGENT_CMD` to your agent CLI — see [`docker-compose.yml`](docker-compose.yml).

### From source

```sh
# 1. Build the UI (once, or after frontend changes)
cd web && npm install && npm run build && cd ..

# 2. Run the server — it serves the API, the live socket, and the built UI
cargo run                          # → http://127.0.0.1:7070

# Frontend dev with hot reload (proxies /api and /ws to the server):
cd web && npm run dev
```

### Organizational memory (optional)

Point Overmind at any MCP memory server exposing `get_context` / `store_memory` / `store_decision` — [Wadachi](https://github.com/EliaCinti/wadachi) is the reference:

```sh
OVERMIND_MEMORY_CMD="BRAIN_DIR=/path/to/brain wadachi" cargo run
```

Agents then load org context before working and record what they learned. Unset it and Overmind runs identically, memoryless.

### Configuration

| Env var | What |
|---|---|
| `OVERMIND_ADDR` | Listen address (default `127.0.0.1:7070`) |
| `OVERMIND_DB` | SQLite URL (default `sqlite://overmind.sqlite`) |
| `OVERMIND_DATA_DIR` | Worktrees & runtime data (default `./overmind-data`) |
| `OVERMIND_AGENT_CMD` | Agent adapter command (default: Claude Code CLI) |
| `OVERMIND_MEMORY_CMD` | MCP memory server command (unset = no memory) |
| `OVERMIND_MEMORY_POOL` | Concurrent memory connections (default `4`) |
| `OVERMIND_HEARTBEAT_SECS` | Scheduler tick (default `30`) |
| `OVERMIND_SESSION_TIMEOUT_SECS` | Kill sessions over this (default `3600`) |
| `OVERMIND_START_ESTIMATE_CENTS` | Budget reservation per start (default `50`) |
| `OVERMIND_WEB_DIR` | Built SPA to serve (default `./web/dist`) |

<br/>

## Status

Pre-alpha, built in the open. The core is done and tested: company & org chart, tasks & board, agent execution in worktrees, heartbeats & recovery, budgets & governance, hash-chained audit, and organizational memory over MCP. Next on the [roadmap](docs/ROADMAP.md): managed per-company brains + memory UI, Overmind as an MCP server, and container-based agent sandboxing.

The design is documented before the code: see [VISION](docs/VISION.md), [ARCHITECTURE](docs/ARCHITECTURE.md), the [UX principles](docs/UX.md), and the [Architecture Decision Records](docs/adr/).

<br/>

## Prior art & credits

Overmind's org layer is inspired by [Paperclip](https://github.com/paperclipai/paperclip) (MIT) and its execution layer by [Vibe Kanban](https://github.com/BloopAI/vibe-kanban). It adopts Paperclip's vocabulary and semantics where they serve (see [PAPERCLIP-ALIGNMENT](docs/PAPERCLIP-ALIGNMENT.md)) and contains **no AGPL code**. The organizational-memory layer and the tamper-evident audit chain are Overmind's own.

## License

MIT — self-hosted, no account, yours.
