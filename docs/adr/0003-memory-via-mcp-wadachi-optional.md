# ADR-0003: Organizational memory via MCP `MemoryProvider`; Wadachi as reference implementation, strictly optional

- **Date:** 2026-07-19
- **Status:** accepted

## Context

Overmind's differentiator is a persistent organizational memory. Elia maintains Wadachi, an MCP memory server. Hard requirement: **the two projects stay independent** — Overmind must be fully usable without Wadachi, and Wadachi without Overmind.

## Decision

Memory is a `MemoryProvider` contract spoken over **MCP** (get_context / recall / store_memory / store_decision). Wadachi is the reference implementation, but any MCP server implementing the surface works. No provider configured → all memory calls are no-ops; provider failure → logged, never fatal. Neither repo imports the other; the protocol is the only coupling.

## Alternatives considered

- **Embed a memory engine inside Overmind** (Aperant's approach) — tighter integration, but kills independence, duplicates Wadachi, and locks users in. Rejected.
- **Direct Python-library integration with Wadachi** — couples languages and release cycles of two projects meant to be independent. Rejected.

## Consequences

- Wadachi gains a second consumer, which pressure-tests its MCP surface — good for both projects.
- Overmind must be designed and *tested* memoryless-first (M7 acceptance includes the unplugged case).
- Multi-agent concurrent access to one brain is new territory for Wadachi (single-user today); any changes land in Wadachi's repo on its own terms, never as Overmind-specific hacks.
