# Vision

## The problem

AI agents are becoming capable workers, but the tooling to run *many* of them is immature. Today you either get:

- **Org-level orchestration without execution depth** — Paperclip gives you org charts, budgets and governance, but agents work without isolated workspaces, diff review, or a real execution UI.
- **Execution depth without organization** — Vibe Kanban gives you worktrees, boards and diff review for coding agents, but no notion of roles, missions, budgets or governance.
- **Autonomy without accountability** — Aperant runs plan→build→validate pipelines, but memory is internal and coupled, and there is no organizational model.

And in **all three**, the organization is amnesiac: every session starts from zero. Decisions, discovered patterns, and past failures evaporate.

## What Overmind is

Overmind is the synthesis, plus the missing piece:

1. **A company model** (from Paperclip's lineage): org chart with roles and reporting lines, missions that cascade into goals and tasks, per-agent budgets with atomic enforcement, approval gates, hire/pause/terminate governance, and a ticket system where every action is an immutable audit event.
2. **Real execution tooling** (from Vibe Kanban's lineage): each agent works in an isolated git worktree with its own branch; work is visible on a kanban board; humans review diffs with inline comments; multiple agents run in parallel safely. Overmind is both an MCP client (agents use tools) and an MCP server (external agents can file tickets and read board state).
3. **A pluggable organizational memory** (the new piece): a `MemoryProvider` interface, speaking MCP, through which the *whole organization* remembers. Agents load context on wake ("what do we know about this?"), store discoveries and decisions on completion, and the org accumulates judgment over time. The first-party brain is **Wadachi**: Overmind can provision and manage a dedicated Wadachi instance per organization, and surfaces its memory in the UI (decisions linked to the tickets that produced them). The interface stays generic — any conforming MCP server works — and Overmind degrades gracefully to full functionality (minus memory) when no provider is configured.

### The two-project contract

Overmind and Wadachi are **separate projects with a privileged integration** (the VS Code + GitHub model — see [ADR-0004](adr/0004-wadachi-first-party-managed-brain.md)):

- Overmind without Wadachi: fully functional orchestrator, no persistent memory.
- Wadachi without Overmind: fully functional personal brain, as today.
- Together: an organization that learns — with a managed, per-organization brain and first-class memory UI. Overmind never touches a user's personal Wadachi brain.

The coupling is MCP plus process management; neither repo imports or vendors the other's code. Development, releases and websites stay separate.

## Pillars (in priority order)

1. **Accountability** — immutable audit log, atomic task checkout, atomic budget enforcement. If we can't prove what an agent did and what it cost, the feature doesn't ship.
2. **Security** — agents run sandboxed with least privilege; secrets never enter agent context; every approval gate is enforced server-side, not by prompt.
3. **Memory** — the differentiator. The memory interface is designed first-class, not bolted on.
4. **Craft** — the UI and the docs are part of the product. "Fatto bene" is a requirement, not an aspiration.

## Non-goals (for now)

- **Not a cloud service.** Self-hosted only; no accounts, no telemetry.
- **Not a model provider.** Overmind orchestrates external agents (Claude Code, Gemini CLI, …); it does not call LLM APIs to "be" the agents itself.
- **Not a chat app.** Agents are asynchronous workers you manage, not chatbots you talk to.
- **Not a Paperclip plugin ecosystem clone.** We build our own surface; feature parity with Paperclip is a reference point, not a checklist.

## First user

Elia. The first real workload is orchestrating the FeyNotes lecture pipeline and university project work. If Overmind isn't useful for its own author, it isn't useful.
