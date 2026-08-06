# ADR-0021: Characterization on two axes (function × domain), a real model, and multimodal as a declared capability

- **Date:** 2026-08-06
- **Status:** accepted

## Context

M14 closes with one slice open: "domain archetypes + multimodal". Its acceptance criterion is a
*"Media & A/V quality"* agent, hired **without free text**, that uses a declared web-research
capability and returns a structured result. Today that agent exists only as a job title written over
a `researcher`, because the built-in catalog ([`db.rs`](../../crates/overmind-server/src/db.rs),
`builtin_archetypes`) is entirely software: `chief-executive`, `security-engineer`,
`backend-developer`, `frontend-developer`, `code-reviewer`, `researcher`, `technical-writer`.

Three facts constrain the design, and measuring them changed it.

**1. The archetype conflates two independent questions.** *"Media & A/V quality"* is a **function**
(reviewing for quality) applied to a **domain** (media and A/V). So is *"Security Engineer"*:
reviewing, applied to security. The catalog encodes those pairs as single rows, so covering a second
domain means re-writing every function for it. [UX.md](../UX.md) sets the bar the naive fix would
break — *"ten excellent archetypes beat thirty mediocre ones"*, and *"if a common need forces a user
into free text, that is a catalog bug"*. One row per pair cannot satisfy both at once: it either
multiplies mediocre rows or leaves the uncovered pairs to free text.

**2. `AgentTraits.model` is decorative.** It is seeded per archetype, patchable through
`TraitsPatch`, versioned under governance — and read by nothing. The adapter command is
`claude -p "$OVERMIND_TASK_PROMPT" --output-format json`, with no `--model`; the only `model` that
reaches the database is `cost.model`, parsed back out of the adapter's own result envelope. So
M15's *"the CEO is on the strongest model"* is not true in production, and the values on hand are
not all real model identifiers: the hire dialog offers `claude-sonnet` / `claude-opus` /
`claude-haiku`, none of which is a model id, while `CEO_MODEL` is `claude-opus-4-8`, which is. This
is precisely the defect [ADR-0005](0005-structured-agent-characterization.md) promised against and
M14's second slice already fixed once for `permissions`: a field that is stored, versioned
and believed, but never enforced.

**3. Every current Claude model is vision-capable.** Checked, not assumed. There is therefore no
model in the catalog that a "multimodal" flag could gate against — a flag that means *"this model
can see"* would be a second decorative field, born the way `permissions` and `model` were.

## Decision

### The archetype is the function; the domain is a second, orthogonal axis

`archetypes` keeps its table and its meaning narrows to **what kind of work the agent does**. A new
`domains` table carries **the field it does it in**, with the same shape (`slug`, `name`,
`description`) plus a **patch** applied on top of the archetype's defaults. `agents` gains a nullable
`domain_id`; absent means the `general` domain, so every existing agent keeps working unchanged.

Characterization composes in one direction, most general to most specific:

```
archetype.default_traits          the function's baseline
  + domain.traits_patch           the field's additions (focus areas, declared capabilities)
  + user TraitsPatch              UX Level 2 "tune"
  = the agent's traits
```

*"Media & A/V quality"* is then `reviewer × media-av`: a hire made of two clicks, no free
text, and the acceptance criterion is met by construction rather than by adding a row for it.

A domain **adds** and never removes: it may contribute focus areas, declared capabilities, and one
line of prompt context about the field. It cannot widen the enforced set — `task:code` /
`task:knowledge` come from the function alone, because what kind of work an agent may be checked out
onto is a property of the function, not of the subject matter.

### The model is chosen by us, from a registry, and actually reaches the adapter

A server-side registry (`model.rs`) is the single place that knows which models exist, which are
vision-capable, and which is the strongest. `AgentTraits.model` is validated against it at hire and
at every traits patch: an unknown model is **refused at the boundary** rather than stored and fed to
a prompt later — the same rule M16 applied to language codes.

The chosen model reaches the adapter as `OVERMIND_AGENT_MODEL`, and the default command becomes
`claude -p "$OVERMIND_TASK_PROMPT" --model "$OVERMIND_AGENT_MODEL" --output-format json`. The
command stays configurable, so a custom adapter is unaffected and receives the variable regardless.
The default currently exists in two copies (`runner.rs` and `ceo.rs`); it becomes one.

### Multimodal is a declared capability, enforced where enforcement is honest

`AgentTraits.multimodal` states that the agent is **expected to work with visual material**. It is
enforced at two points, and neither pretends to be more than it is:

- **At hire and patch** — a multimodal agent must sit on a vision-capable model. Every model in
  today's registry is vision-capable, so this check is currently vacuous. It is written anyway,
  because the registry is where that fact lives and a model without vision is a plausible next
  entry; a rule that is true by luck should still be stated by the code that depends on it.
- **At checkout** — a task carrying image inputs may only be handed to a multimodal agent. This one
  is load-bearing today, and it is the same kind of rule as `task:code`: not a claim about what the
  spawned CLI *can* do, but a refusal to hand an agent work it was never characterized for.

This keeps M14 slice 2's two families intact — enforced where the server genuinely decides,
declared where it does not.

### Catalog prose is translated by slug

`archetypes.name` / `.description` and the new `domains` equivalents are rendered today straight from
the database, in English, inside an interface M16 made fully Italian. Built-in rows are translated
**by slug** in `lib/i18n.ts`, with the stored prose as the fallback for rows a user or a future
plugin adds. This is M16 slice D's rule applied to catalog data: the server sends the identity, the
client writes the words.

## Alternatives considered

- **Add domain rows to the existing catalog** (`av-engineer`, `financial-analyst`, …). The literal
  reading of the roadmap, no schema change, no ADR. Rejected: the catalog grows by multiplication,
  and every function×domain pair nobody anticipated falls back to free text — which UX.md defines as
  a catalog bug, so the cheap option buys its cheapness by manufacturing the exact defect the
  milestone exists to remove.
- **A free-text `domain` string on the agent.** One migration, no catalog. Rejected: it is Level 3
  wearing a structured field's clothes — unvalidatable, untranslatable, and impossible to compose a
  traits patch from. ADR-0005 rejects prompt-only characterization for these reasons.
- **A separate `capabilities` field for multimodal**, parallel to `permissions`. Rejected for the
  reason slice 2 already gave when it declined to add a `capabilities` field beside
  `permissions`: a second field describing the same thing leaves the first one lying.
- **Gate multimodal on model capability only**, with no checkout rule. Rejected: with every model
  vision-capable it would enforce nothing at all, which is how `model` and `permissions` got here.

## Consequences

- **Migration 0017** adds `domains`, `agents.domain_id`, and `multimodal` to the traits JSON.
  Existing agents read as `general` domain, non-multimodal — no backfill, no behavior change.
- **The CEO's proposal gains a field.** `org.rs` parses and validates `domain` alongside `archetype`
  for each proposed member, and the proposal prompt must describe the two axes. An unknown domain is
  refused exactly like an unknown archetype: a proposal you cannot accept is worse than no proposal.
- **`CEO_MODEL` becomes a registry lookup** rather than a constant, so "the strongest model" stays
  true as the registry moves instead of being true on the day it was typed.
- **The hire dialog gains a step** — function, then domain, then tune. Progressive disclosure holds:
  the domain step defaults to `general` and can be passed through in one click.
- **What this does not do:** it does not make declared capabilities enforceable. `web:read` on a
  media agent is still compiled into the prompt and not policed, because we still shell out to an
  external CLI. That remains M10.
