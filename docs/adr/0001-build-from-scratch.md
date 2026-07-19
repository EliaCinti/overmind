# ADR-0001: Build from scratch, study Paperclip and Vibe Kanban as references

- **Date:** 2026-07-19
- **Status:** accepted

## Context

Overmind's feature target is Paperclip's org layer + Vibe Kanban's execution layer + a pluggable memory layer. Both Paperclip (MIT) and Vibe Kanban exist and work; forking either would be faster initially.

## Decision

Build Overmind from scratch. Use Paperclip as the **design reference** (feature semantics, atomicity guarantees) and Vibe Kanban as the **pattern reference** (Rust worktree/runner/MCP patterns). Porting isolated code from MIT sources with attribution is allowed; wholesale forking is not.

## Alternatives considered

- **Fork Paperclip** — inherits a Node architecture not designed for worktree isolation or a first-class memory interface; the codebase stays "theirs"; retrofitting would likely cost more than building right. Rejected.
- **Fork Vibe Kanban** — has the execution layer but no org model; same retrofit problem in reverse. Rejected.

## Consequences

- Slower start; architecture designed for all three pillars from day one; the codebase is fully owned and understood.
- **License rule:** Aperant is AGPL-3.0 — *ideas only, never code*. Mixing AGPL code would force the whole project to AGPL and kill the MIT license.
