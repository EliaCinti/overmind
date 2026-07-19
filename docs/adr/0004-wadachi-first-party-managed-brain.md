# ADR-0004: Wadachi as first-party managed brain

- **Date:** 2026-07-19
- **Status:** accepted (extends ADR-0003; does not supersede it)

## Context

ADR-0003 made memory a generic, optional `MemoryProvider` over MCP, with Wadachi as one reference implementation among possible others. Elia decided to deepen the relationship: Wadachi should be *integrated into* Overmind as its official brain — while both projects remain separate repos, separate products, separate websites.

## Decision

Wadachi is promoted from "reference implementation" to **first-party managed brain**:

1. **Managed lifecycle** — Overmind can provision, launch, supervise and shut down a Wadachi instance on its own: one click and a company has a brain. Each company gets its **own dedicated brain directory** (e.g. `<data-dir>/companies/<company>/brain/`). Overmind never reads or writes a user's personal Wadachi brain.
2. **First-class UI** — organizational memory is surfaced inside Overmind: memory browser, decisions linked to the tasks that produced them, "why did agent X do Y" tracing back to the memories that guided it.
3. **The contract stays open** — the `MemoryProvider` MCP interface from ADR-0003 is unchanged and generic; any conforming MCP server works; no provider configured → full functionality, memory calls are no-ops.
4. **Development stays separate** — no vendored/embedded Wadachi code in this repo. Integration is via MCP + process management only. Changes Wadachi needs to serve Overmind (notably **concurrent multi-agent access** — Wadachi is single-user today) are developed in the Wadachi repo on its own terms.

The model is VS Code + GitHub: separate products, privileged integration.

## Alternatives considered

- **Embed/vendor Wadachi code inside Overmind** (Aperant's approach) — duplicated code, coupled release cycles, every fix done twice. Rejected firmly.
- **Keep Wadachi as just-a-reference (ADR-0003 as-was)** — clean but forfeits the deepest differentiator (memory UI, one-click brain) and gives Wadachi no showcase. Rejected in favor of first-party.

## Consequences

- Roadmap: memory work splits into M7 (contract + basic integration) and M8 (managed brain + memory UI); later milestones renumbered.
- Wadachi gains a hard new requirement on its own roadmap: safe concurrent access from N agents.
- Overmind must still be designed and tested memoryless-first (unchanged from ADR-0003).
- Marketing/positioning: "the orchestrator that ships with a brain" — and every Overmind user discovers Wadachi.
