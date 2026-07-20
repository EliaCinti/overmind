# ADR-0013: Organizational memory over MCP — the Wadachi integration

- **Date:** 2026-07-20
- **Status:** accepted (implements ADR-0003, extends toward ADR-0004)

## Context

M7 is the differentiator: the organization remembers across sessions. ADR-0003 fixed the shape — a `MemoryProvider` spoken over MCP, Wadachi as reference, strictly optional with graceful degradation. This ADR records how it's actually built.

## Decisions

1. **MCP over stdio, one persistent connection.** `Memory` speaks JSON-RPC 2.0 over a spawned process's stdio. The server is spawned and handshaked (initialize → notifications/initialized) **once**, then reused for every call (`tools/call` with a monotonic id) for its lifetime. Calls are **serialized by a mutex** — which also shields a single-user memory server (Wadachi today) from concurrent access, so the M8 concurrency requirement doesn't block M7. Any error (broken pipe, timeout, protocol) **drops the connection**; the next call transparently re-spawns and re-handshakes. This avoids paying the memory server's cold start (≈1s for Python/Wadachi) on every call — which matters once calls grow beyond the start/finish pair (resume re-loads context, `store_decision`, future `recall` during work).
2. **Config, not code coupling.** `OVERMIND_MEMORY_CMD` is the shell command that launches the memory server (e.g. the `wadachi` binary, which runs an MCP server on stdio with no args). Unset → memory disabled. Overmind never imports Wadachi; the only coupling is the protocol — exactly ADR-0003/0004.
3. **Best-effort, never fatal.** Every memory call is wrapped in a timeout (30s) and swallows all errors (spawn/timeout/protocol) with a log line. A missing, broken, or slow memory server cannot fail a task. Verified by a test that points `OVERMIND_MEMORY_CMD` at `exit 7` and still completes the task.
4. **The loop.** On start, `get_context(cwd, task_description)` — its text is injected into the agent's prompt ("What the organization remembers…") and exposed as `OVERMIND_MEMORY_CONTEXT`. On successful finish, `store_memory(title, content, project, tags, category)` records what was done. `get_context` sends only `cwd` + `task_description` to match Wadachi's schema; **per-company isolation comes from the brain directory, not an argument** (`Memory::with_brain_dir` sets `BRAIN_DIR`, wired up by the managed brain in M8).

## Alternatives considered

- **Per-call spawn** (spawn + handshake + one tool + exit, every call) — simplest and stateless, but pays the server's cold start (~1s for Python/Wadachi) on every call. Fine at 2 calls/session, wasteful once calls grow (resume, decisions, recall). Rejected in favor of the persistent connection.
- **Making Wadachi multi-user so Overmind can drop the mutex and issue concurrent calls** — the serialization is *not* the cost (memory calls are infrequent and dwarfed by LLM latency); the cold start was, and the persistent connection already fixed that. Concurrent-safe brain writes are a real, separate effort that belongs in the **Wadachi repo** (ADR-0004; logged as a Wadachi requirement). Premature here. When evidence demands concurrency, a small connection *pool* replaces the single mutexed connection — but only after Wadachi is concurrency-safe, since a pool would otherwise hammer a single-user brain.
- **A concurrent request-correlation client** (one connection, many in-flight requests keyed by id, a background reader task) — needed only once Wadachi is multi-user; until then it would just expose the single-user brain to races. Deferred with the above.
- **HTTP/SSE MCP transport** instead of stdio — Wadachi is a stdio server; stdio is the native, zero-config path for a locally-spawned brain.
- **Passing `project` to get_context** — not in Wadachi's tool schema (it derives project from cwd); sending it risks a validation error. Dropped; brain-dir scoping supersedes it.
- **Making memory writes transactional with the session** — they're external I/O; keeping them outside the DB transaction (after commit, best-effort) is correct and keeps the audit chain independent of the brain.

## Consequences

- Overmind is designed and *tested* memoryless-first: the no-provider path is a first-class test, and the two-project independence of ADR-0003/0004 holds.
- Verified against **real Wadachi** (throwaway brain): task 1's completion was stored, and task 2's agent received it via real `get_context` — the "avoid a past mistake" loop, end to end.
- M8 (managed brain) builds directly on `Memory::with_brain_dir`: Overmind will provision `<data-dir>/companies/<company>/brain/` and launch Wadachi with that `BRAIN_DIR`, giving each company an isolated memory — no code change to this layer, just lifecycle management.
