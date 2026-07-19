# ADR-0005: Structured agent characterization (archetypes + traits), free text as additive expert layer

- **Date:** 2026-07-19
- **Status:** accepted

## Context

Elia set a product requirement before M1: world-class UX — users must reach complex results with zero friction, customizing agents mostly by clicking, with a deeper free-form level for unusual needs (see [UX.md](../UX.md)). Meanwhile, the security pillar requires that what an agent may do is enforced server-side, not suggested via prompt.

## Decision

An agent's characterization is **structured data first**:

```
Agent
├── archetype        e.g. "security-engineer" — a seedable data object, not hardcoded UI
├── traits           focus areas, tool/permission grants, autonomy level,
│                    review strictness, budget, model — all typed & validated
└── custom_brief     optional free-form instructions (Level 3), additive only
```

- Archetype + traits compile into both the agent's *prompt context* and its *enforced configuration* (permissions, tools, budget, gates). One source of truth for both.
- `custom_brief` is appended to the compiled context but **can never override enforced configuration**: a brief saying "push directly to main" does nothing if the traits don't grant it.
- Characterization is versioned config under governance (revisioned, roll-backable), like everything else.

## Alternatives considered

- **Prompt-only characterization** (one big textarea, the common approach) — simple to build, but unenforceable (security by prayer), intimidating for users, and impossible to build a guided UI on. Rejected.
- **Structured-only, no free text** — clean and safe but caps power users; "strano strano" use cases become impossible. Rejected: Level 3 exists, clearly fenced.

## Consequences

- M1's domain model gains `Archetype` as an entity and structures the `Agent` record accordingly (this ADR predates M1 by design — retrofitting structure onto a prompt string later would be painful).
- The archetype gallery ships as seed data; users and future plugins can add archetypes without touching UI code.
- UI work (M5) becomes tractable: every control maps to a typed field.
