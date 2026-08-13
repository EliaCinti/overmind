# ADR-0027: Agents reach memory through Overmind, not through the filesystem

- **Date:** 2026-08-13
- **Status:** accepted

## Context

[ADR-0015](0015-agent-facing-memory-tools.md) promised agents that call `recall` and `why`
themselves. M8 slice 3 has been open on that promise since July, blocked on a prerequisite
that [ADR-0026](0026-change-awareness-across-concurrent-agents.md) showed never existed.
With that cleared, the obvious implementation is to hand each agent a Wadachi over stdio
with `BRAIN_DIR` pointed at its company's brain — the same shape a human's own Wadachi runs
in, and one the measurements in ADR-0026 show holds under real concurrency.

**It does not work, and the reason is our own security boundary.**

[ADR-0023](0023-os-level-sandboxing.md) put every caged run behind a `(deny default)`
`sandbox-exec` profile. The writable set is the run directory, `/private/tmp`, this
session's `TMPDIR`, the adapter's own paths under `$HOME`, and whatever `sandbox_allow`
adds. Reads are allowed for system paths only. **The company's brain directory
(`<data-dir>/companies/<id>/brain/`) appears in neither list.** An agent that spawned
Wadachi as a subprocess would spawn it *inside* the cage, where it can neither read nor
write the brain it was pointed at.

So slice 3 costs a decision about the cage, not a config file. Three ways out were weighed
(see Alternatives); this ADR takes the one that does not spend the boundary.

## Decision

**Agents do not touch the brain. They talk to Overmind, which talks to the brain.**

Overmind gains an MCP server surface of its own. A run's agent receives one MCP server in
its config — Overmind — and through it the read tools ADR-0015 promised.

### The boundary that matters: agents read, Overmind writes

The exposed set is **read-only against memory**: `recall`, `why`, and the two positional
tools ADR-0026 added, `brain_watermark` and `changed_since`. `store_memory` and
`store_decision` are **not** exposed.

This is not caution, it is ADR-0015 decision 3 kept intact: the completion-time write stays
orchestrator-authoritative *"with the task as provenance"*, and ADR-0025 made that
provenance a real `memory_links` row. An agent that could write directly would produce
memories with no task behind them, and the UI that answers "which task produced this?" would
start answering "nothing" — quietly, and only for the memories written the new way.

### Transport: HTTP, because Overmind already is an HTTP server

Overmind is an axum server on a port. Exposing MCP over that same listener adds an endpoint,
not a process, a lifecycle, or a second thing that can be down while the first is up. The
alternative — a stdio shim per run that proxies to the same place — buys nothing and adds a
process to supervise.

The cage already permits `network*` and `system-socket`, so no sandbox change is needed.
**That is the whole point of this decision:** ADR-0023's profile is untouched.

### The token is the identity, not just the lock

Each run gets a token minted at checkout, stored on its session row, and invalidated when
the run ends. It is written into a per-run MCP config file — mode `0600`, deleted in a path
that covers the run's whole lifetime, including failure and timeout — exactly as ADR-0015
required, and never into a config shared with other agents.

Its job is not only to keep other local processes off the endpoint. **It is what tells
Overmind who is calling.** A request carries no company id and no brain path; Overmind
resolves those from the token. An agent cannot name another company's memory because it
cannot name a company at all.

That is the property option A could not have offered. Filesystem isolation says "this
process may write these bytes"; it cannot say "this agent may read its own company's
memories and nothing else, and here is the record of what it asked". A mediated endpoint is
where a limit or an audit line can exist at all. We are not adding either today — but a
design that forecloses them is a design we would have to undo.

### Relationship to M9

M9 is "Overmind as MCP server: expose tasks/board/audit over MCP; external agents can file
and read tasks." This is that server, built for its first consumer. M9 then becomes more
tools and an external caller on infrastructure that already exists and is already exercised
by every caged run, rather than a greenfield endpoint whose first user is a stranger.

Slice 3 therefore lands **inside** M9's foundation. The roadmap records it that way instead
of pretending two servers were built.

### Degradation stays as it always was

No memory provider → the tools are absent from the config and the agent never sees them, the
same way `OVERMIND_MEMORY_CONTEXT` is empty today (ADR-0003, rule 6). An endpoint that is
up but whose provider is down answers with an error the agent can read, and the run
continues: memory never breaks a task.

## Alternatives considered

**Widen the cage to the company's brain directory.** The direct reading of slice 3: add
`<data-dir>/companies/<id>/brain/` to the writable set and hand the agent a stdio Wadachi.
Little code, and the isolation is real — slice 1 already gives each company its own
directory. Rejected: it spends a deroga on ADR-0023, three weeks old, to buy a capability
option B provides with no deroga at all. It also puts *write* access to organizational
memory inside the cage, when the agent is not supposed to write memory in the first place —
the grant would be wider than the need, and wider grants are the thing that profile exists
to refuse. And it leaves no place to ever put a limit or an audit line.

**Run Wadachi outside the cage, reached over a socket.** This is `wadachi serve-http`, which
does not exist (ADR-0026 withdrew the claim that it does). Building a long-lived
authenticated endpoint inside Wadachi to serve one consumer, when the consumer already has
an HTTP server, is the wrong repository doing the wrong work.

**Give agents `store_memory` too, while we are here.** Rejected: it silently breaks the
provenance chain ADR-0015 and ADR-0025 built. If agent-authored memories are wanted later,
they need a provenance story first, and that is its own decision.

**Wait for M9 and do slice 3 after.** Rejected as an ordering, not as a direction: it is the
same work, and doing it now gives M9's server a first consumer that is fully under our
control, before an external caller depends on its shape.

## Consequences

**Easier.** Agents get the read tools ADR-0015 promised without touching ADR-0023's profile.
M9 starts with a working, exercised MCP server instead of an empty one. Per-company scoping
becomes something the server enforces rather than something the directory layout implies —
which also means it can be tested by asking for the wrong company and being refused, rather
than by inspecting paths.

**Harder.** Overmind now speaks MCP in both directions, and the two must not be confused in
the code: `mcp.rs` is the client, and the server surface is its own module. There is a token
to mint, store, hand over, and invalidate, plus a per-run file whose deletion has to survive
every exit path — the failure mode ADR-0015 flagged, and the one to write a test for rather
than a comment about.

**Committed to.** Agents read; Overmind writes. Requests carry a token, never a company id.
The sandbox profile stays as ADR-0023 wrote it, and any future proposal to widen it for
memory has to argue against this ADR first.
