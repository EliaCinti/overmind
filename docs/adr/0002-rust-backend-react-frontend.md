# ADR-0002: Rust backend (axum + SQLite), React + TypeScript frontend

- **Date:** 2026-07-19
- **Status:** accepted

## Context

The server's core jobs are concurrency-heavy and correctness-critical: spawning and supervising many agent processes, managing git worktrees, atomic task checkout + budget reservation, an append-only audit log, WebSocket streaming. Elia asked to optimize for the language that *performs best for this domain*, not for learning-curve convenience (Claude writes the code).

## Decision

- **Backend:** Rust — axum (HTTP/WS), tokio (async), sqlx + SQLite (storage).
- **Frontend:** React + TypeScript + Vite, typed API client generated from OpenAPI.

## Alternatives considered

- **TypeScript full-stack (Paperclip's stack)** — one language, source-compatible with the main reference. Rejected: weaker guarantees for exactly the parts that must not fail (concurrency, atomicity), and the performance ceiling is lower for process supervision at scale.
- **Python backend** — consistent with Wadachi. Rejected: worst fit for concurrent process supervision; coupling to Wadachi's language is a non-goal (the projects are independent by design).

## Consequences

- Vibe Kanban (same stack) becomes directly usable as a pattern reference.
- Compile-time guarantees where they matter most (state machines, concurrency).
- Two languages in the repo (Rust + TS) — accepted cost, standard for this architecture.
