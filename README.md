# Overmind

> The mind that runs your agent company.

Overmind is an open-source orchestrator for **teams of AI agents**. It organizes agents into a company — org chart, missions, budgets, governance — and gives each agent real execution tooling: isolated git worktrees, a kanban board, diff review, full audit trails.

What makes Overmind different: it is **memory-native**. Through a pluggable memory interface (MCP), the whole organization shares a persistent brain — decisions with their *why*, discovered patterns, past mistakes — that survives across sessions. Overmind ships with first-party integration for [Wadachi](https://github.com/EliaCinti/wadachi): one click and the organization has its own managed brain. The interface stays open (any MCP memory server works) and Overmind works fully without one.

**Status: pre-alpha — design phase.** Nothing runs yet. Start with the docs:

| Document | Purpose |
|---|---|
| [docs/VISION.md](docs/VISION.md) | What we're building, why, and what we're *not* building |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design (living draft) |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones — the single source of truth for "what's next" |
| [docs/adr/](docs/adr/) | Architecture Decision Records — every significant choice, with rationale |

## Principles

1. **Self-hosted, no account, yours.** Your agents, your data, your machine.
2. **Nothing untracked.** Every task, tool call and decision is an immutable audit event.
3. **Memory is a plugin, not a lock-in.** Overmind runs without a brain; with one, the org learns.
4. **Quality over breadth.** Fewer features, built impeccably.

## License

MIT. Overmind is inspired by [Paperclip](https://github.com/paperclipai/paperclip) (MIT) and [Vibe Kanban](https://github.com/BloopAI/vibe-kanban); it contains **no AGPL code** (ideas from Aperant are welcome, its code is not — see [ADR-0001](docs/adr/0001-build-from-scratch.md)).
