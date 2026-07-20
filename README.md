# Overmind

> The mind that runs your agent company.

Overmind is an open-source orchestrator for **teams of AI agents**. It organizes agents into a company — org chart, projects, budgets, governance — and gives each agent real execution tooling: isolated git worktrees, a kanban board, diff review, full audit trails.

What makes Overmind different: it is **memory-native**. Through a pluggable memory interface (MCP), the whole organization shares a persistent brain — decisions with their *why*, discovered patterns, past mistakes — that survives across sessions. Overmind ships with first-party integration for [Wadachi](https://github.com/EliaCinti/wadachi): one click and the organization has its own managed brain. The interface stays open (any MCP memory server works) and Overmind works fully without one.

**Status: pre-alpha — design phase.** Nothing runs yet. Start with the docs:

| Document | Purpose |
|---|---|
| [docs/VISION.md](docs/VISION.md) | What we're building, why, and what we're *not* building |
| [docs/UX.md](docs/UX.md) | UX principles: progressive disclosure, click-first, enforceable choices |
| [docs/PAPERCLIP-ALIGNMENT.md](docs/PAPERCLIP-ALIGNMENT.md) | Vocabulary canon and recorded deviations from Paperclip |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design (living draft) |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones — the single source of truth for "what's next" |
| [docs/adr/](docs/adr/) | Architecture Decision Records — every significant choice, with rationale |

## Running it

```sh
# 1. Build the UI (once, or after frontend changes)
cd web && npm install && npm run build && cd ..

# 2. Run the server — it serves the API, the live socket, and the built UI
cargo run                      # → http://127.0.0.1:7070

# Frontend dev with hot reload (proxies /api and /ws to the server on :7070):
cd web && npm run dev
```

Key env vars: `OVERMIND_DB`, `OVERMIND_DATA_DIR`, `OVERMIND_AGENT_CMD` (agent adapter command), `OVERMIND_WEB_DIR`, `OVERMIND_ADDR`, `OVERMIND_HEARTBEAT_SECS`, `OVERMIND_SESSION_TIMEOUT_SECS`.

## Principles

1. **Self-hosted, no account, yours.** Your agents, your data, your machine.
2. **Nothing untracked.** Every task, tool call and decision is an immutable audit event.
3. **Memory is a plugin, not a lock-in.** Overmind runs without a brain; with one, the org learns.
4. **Quality over breadth.** Fewer features, built impeccably.

## License

MIT. Overmind is inspired by [Paperclip](https://github.com/paperclipai/paperclip) (MIT) and [Vibe Kanban](https://github.com/BloopAI/vibe-kanban); it contains **no AGPL code** (ideas from Aperant are welcome, its code is not — see [ADR-0001](docs/adr/0001-build-from-scratch.md)).
