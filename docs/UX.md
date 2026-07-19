# UX Principles

> UX is a pillar, not a coat of paint. These principles bind every screen and every API that backs one. If a feature can't be exposed without violating them, the feature isn't ready.

## The principles

1. **Zero to working in minutes.** Everything has a sensible default. A new user reaches a running organization with agents doing real work without reading docs.
2. **Progressive disclosure.** Three levels, always in this order:
   - **Level 1 — Pick:** curated choices, one click, fully working result.
   - **Level 2 — Tune:** structured customization — toggles, chips, sliders, dropdowns. Writing is minimized; clicking is the norm.
   - **Level 3 — Expert:** free-form power — custom prompts, raw config, custom MCP servers. Never required, always available, clearly separated.
3. **Click first, type last.** If a choice can be an option, it must not be a text field. Free text is the escape hatch, not the interface.
4. **The system guides.** Live previews ("this agent will review every PR touching `auth/`"), suggestions based on what the org already has, validation *before* mistakes instead of errors after.
5. **Structured choices are enforceable — and that's the point.** Every Level 1/2 option maps to server-enforced configuration (permissions, tools, budgets, gates) — never to mere prompt text. Level 3 free text can *add* behavior but can **never override** server-enforced limits. This is where UX meets the security pillar.
6. **Complexity must feel earned, not imposed.** Reaching something complicated should feel like descending stairs, not hitting a wall. No screen exposes an option the user's current task doesn't need.
7. **Beauty is a requirement.** Coherent visual language, purposeful motion, dark/light parity. "Works but ugly" does not ship.

## Reference flow: hiring an agent

The canonical example of the three levels (drives the M5 UI and the M1 data model):

- **Level 1 — Archetype gallery.** Cards: *Security Engineer*, *Backend Developer*, *Frontend Developer*, *Code Reviewer*, *Researcher*, *Technical Writer*, *DevOps*… One click = a fully-formed agent: preconfigured focus, tools, permissions, review strictness, default budget. Example: *Security Engineer* ships attentive to OWASP-class issues, with read access to the whole codebase, write access to nothing without review.
- **Level 2 — Tuning.** Structured controls on top of the archetype: focus-area chips (e.g. `auth`, `dependencies`, `secrets handling`), tool and permission toggles, autonomy level (propose-only → act-with-approval → act-freely-within-budget), review strictness, budget slider, model choice. A live summary panel restates in plain words what the agent will and won't do.
- **Level 3 — Expert mode.** A clearly separated tab: free-form instructions appended to the structured characterization, custom MCP servers, raw config (versioned, roll-backable like all governance config). A visible note states what remains enforced regardless of the text.

## The built-in catalog is a product surface

The archetypes and traits Overmind ships with must be **complete and excellent**, covering the widest practical range of what users actually need:

- **Breadth:** the structured levels (1–2) must satisfy the large majority of real use cases on their own. Level 3 exists for the genuinely exotic — it is **never an excuse for catalog gaps**. If a *common* need forces a user into free text, that is a catalog bug and gets filed and fixed as one.
- **Depth:** every shipped archetype is a curated product artifact — thought-through defaults, sensible permission boundaries, a written preview of behavior — not a stub with a name. Ten excellent archetypes beat thirty mediocre ones; the catalog grows, but never below this bar.
- **Trait coverage:** the trait taxonomy (focus areas, permissions, autonomy, strictness, budget, model) must be expressive enough to meaningfully differentiate agents *within* the same archetype.

## Consequences for the backend

- Agent characterization is **structured data first** (archetype + traits), free text second — see [ADR-0005](adr/0005-structured-agent-characterization.md). The domain model (M1) stores it that way.
- Every UI control corresponds to a typed, validated API field; the API rejects what the UI would not allow.
- Archetypes are data (seedable, extensible), not hardcoded UI.
