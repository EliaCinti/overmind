# ADR-0026: Change awareness across concurrent agents

- **Date:** 2026-08-12
- **Status:** accepted

## Context

When several agents work a company's tasks at once, each one reads the brain at the
start and writes to it at the end. Between those two moments the brain moves — and
nothing tells anybody. Two agents can reach opposite conclusions about the same thing,
both store them, and the organization ends up holding a contradiction that no one
noticed. The promise of ADR-0004 is an org that *learns*; an org that quietly records
mutually exclusive decisions has not learned anything.

### What was measured first (2026-08-12)

Before designing, the storage layer was measured rather than assumed — 8 separate
processes writing one **freshly provisioned** brain, then 4 separate reader processes:

| | |
|---|---|
| concurrent writes on a new brain | **8/8 succeeded**, 0.07 s |
| readers seeing every write, immediately | **4/4 saw all 8** |
| cost of one `recall` | ~0.6 s · embedding model load 0.11 s · embed 3 ms |

So **visibility is not the problem**. Wadachi 0.14.1 holds: there is no in-process data
cache, every search re-reads SQLite, and a memory written by one process is visible to
the next process that looks. What is missing is not the ability to see a change — it is
any reason to look.

### A premise in our own documents is false

[ADR-0024](0024-managed-per-company-brain.md) states *"Wadachi 0.14 ships `wadachi
serve-http`"*, and [ROADMAP](../ROADMAP.md) M8 slice 3 builds on it. **It does not
exist.** `grep -rn "serve-http"` over the Wadachi repository returns nothing; the CLI
offers `init`, `doctor`, `sleep`, `obsidian`, `export`, `restore`, and the MCP server is
stdio only. `wadachi/web.py` is a read-only graph viewer on `http.server`, not an
endpoint.

That claim is **withdrawn here**. It matters twice: slice 3 was blocked on a
prerequisite that was never built, and — since the measurement above shows stdio holds
under the concurrency Overmind actually creates — slice 3 does not need it. Each agent
can receive Wadachi over stdio with `BRAIN_DIR` pointed at its company brain, exactly as
a human's own Wadachi is configured.

This is the third time in this project that a capability has been believed on the
strength of a document rather than a repository. The rule from the 2026-08-09 session
stands: *a changelog is a claim, not a proof.*

### What Wadachi already has, and why it is not enough

`beliefs.py` (`BeliefReviewer.scan`) already detects **supersession** from typed edges,
and `graph.py` already classifies an edge as `updates`, `contradicts`, `cites` or
`relates`. But those edges are derived from a `[[wikilink]]` plus nearby keywords: they
fire only when the writer **already knew** about the other memory and chose to point at
it. They detect *declared* conflict. The case that matters here is the **undeclared**
one — two agents who never saw each other.

`reflect.py` finds non-obvious connections, and both run only inside `sleep()`, a batch
pass that has not run in 28 days. They are archivist's tools. This decision needs an
operating-room tool.

### The trap in the obvious solution

The obvious answer is to notify agents when the brain changes. It is the wrong
primitive. An agent has finite context; a system that tells everyone about everything
makes the memory *worse*, because the signal that mattered arrives buried in signal that
did not. An agent working on billing must not be interrupted because someone changed a
CSS token.

The question is not *how do we notify*. It is: **when must a change actually alter
someone's behaviour?** The answer is narrow — only when it collides with what that agent
is relying on right now. Everything else the agent will find with `recall`, when it
needs it.

## Decision

Do not build notification. Build **collision detection at the moment of writing**,
scoped to the window since the other agents started.

Two mechanisms, the second resting on the first. A third (a *living brief* that is
marked dirty and patched mid-task) is **deliberately deferred** until these two have run
against real work.

### A — Watermarks

The brain gets a cheap, monotonic position, and every unit of work remembers where it
started.

- Wadachi exposes `brain_watermark(project=None)` → the current `MAX(id)` of `memories`
  and `decisions`. O(1), no embeddings.
- Wadachi exposes `changed_since(watermark, project, limit)` → the rows above that
  position. A plain indexed range scan; `memories.id` and `decisions.id` are
  `INTEGER PRIMARY KEY AUTOINCREMENT` and `project` is indexed.
- Overmind records the watermark on the task **at checkout**, in the same transaction
  that claims the task and its budget — so the atomicity invariant already guaranteed by
  M6 covers it.

This is sound because writes serialize: since 0.14.1 every write transaction is
`BEGIN IMMEDIATE`, so there is one writer at a time and id order is commit order. A
reader that has observed watermark *W* can never later be overtaken by a row below *W*.

On its own, A already answers *"what has changed in my project since I started?"* — and
that alone is worth shipping.

### B — Collision candidates, reported at write time

When an agent stores a memory or a decision, Wadachi compares the new item **only**
against the window `id > watermark AND project = <same>` — the handful of things written
since that agent began, not the whole brain — using the embeddings and cosine similarity
already in `search.py`. Items above a similarity threshold are returned in the write's
response as **collision candidates**.

Two boundaries define this design, and both are deliberate:

**Wadachi reports; it does not decide, block, or reject.** It returns candidates and
stores the memory regardless. This keeps the propose-never-auto-accept discipline that
`beliefs.py` and `reflect.py` already follow, and keeps agent identity, task state and
policy out of a memory server that has no business knowing them.

**Overmind decides.** It receives the candidates and tells the human, naming both sides.

> **Correction, made while implementing (2026-08-13).** This paragraph originally said
> Overmind would raise an **approval gate**. Building it showed that to be wrong. An
> approval gates an action that has not happened yet; by the time a collision is known,
> both writes are already in the brain and the task is finished, so there is nothing left
> to authorize. An approval whose only outcome is "seen" is a to-do list wearing
> governance's clothes — and worse, it teaches people to click through the gate that does
> matter. Shipped as a notification (`memory.collision`) carrying both sides and no
> `approval_id`. If a real action ever emerges — marking one memory as superseding the
> other — that action earns its own gate; being told does not.

**We are honest about what this detects.** Cosine proximity is *not* contradiction: two
memories can be nearly identical and agree completely. Phase B detects **proximity under
concurrency** — someone else touched this, while you were not looking — and escalates to
a human. That escalation is the design, not a fallback: telling agreement from
contradiction cheaply and reliably is not something a similarity score can do, and
pretending otherwise would produce a system that silently resolves conflicts wrongly,
which is worse than the problem it replaces.

### Ownership boundary

Rule 6 stands: no Wadachi code is vendored into Overmind; integration stays MCP plus
process management.

| Wadachi | Overmind |
|---|---|
| `brain_watermark`, `changed_since` | records the watermark at checkout |
| similarity over the window, candidate list | knows who works on what |
| stores regardless of candidates | raises the approval gate, surfaces both sides |

The watermark is **passed in** by Overmind on each write. Wadachi never learns who is
writing or when they started — it answers a question about a range of ids.

### The optional-provider rule survives

Per ADR-0003 and rule 6, memory is optional and the org must work without it. A provider
that does not implement `brain_watermark` simply returns nothing: no watermark, no
window, no candidates, and the organization runs exactly as it does today. Change
awareness is an enhancement over a working system, never a dependency of one.

## Alternatives considered

**Broadcast every change to every agent.** Rejected: destroys the agents' context budget
and trains them to ignore the channel. The signal that mattered arrives buried.

**Poll the brain on every tool call.** Rejected: the same relevance problem, paid for
repeatedly. Without a watermark, "what is new" costs a full scan each time.

**Rely on the existing `contradicts` edge.** Rejected: it is derived from a wikilink and
nearby keywords, so it fires only when the writer already knew about the other memory.
It cannot see the undeclared collision, which is the entire problem here.

**Run `BeliefReviewer` / `Reflector` more often instead.** Rejected as the primary
mechanism: they are whole-brain batch passes with no notion of *since when* and no notion
of *who*. They remain valuable, and `sleep()` should be run — but consolidation after
the fact is not the same as catching a collision while both agents are still working.

**Have an LLM adjudicate each candidate pair.** Rejected **for now**: it puts a paid call
on the write path of every memory, and the write path is the one thing that must stay
cheap and always succeed. Worth revisiting if the approval gates prove too noisy in
practice — that is a measurement we do not have yet.

**Build `wadachi serve-http` first**, as ADR-0024 assumed. Rejected: it does not exist,
and the measurement shows stdio holds under the concurrency Overmind creates. Building a
long-lived authenticated endpoint is a real project; it should be justified by a measured
need (pool churn showing as latency, or supervision requirements), not by a sentence in
an ADR that was never true.

**Living brief, marked dirty and patched mid-task (mechanism C).** Deferred, not
rejected. It is the mechanism that would make an agent *act* on a change rather than be
told at the end, and it is where this should eventually go. But it needs A and B beneath
it, and it needs evidence about how often collisions actually happen before we design an
injection path into a running agent's context.

## Consequences

**Easier.** Two agents contradicting each other stops being invisible: it becomes a gate
with both sides named. "What changed since I started" becomes an O(1) question. Slice 3
of M8 is unblocked, and cheaper than planned — stdio, no new server.

**Harder.** The memory contract (ADR-0003) grows an optional watermark argument on
writes, and the checkout transaction grows a field. Overmind gains a new gate reason,
which is one more thing that can interrupt a run — the threshold will need tuning against
real work, and a threshold set too low turns a useful gate into an ignored one.

**Committed to.** Wadachi reports, Overmind decides — never the reverse. Detection is
honest about being proximity rather than contradiction. And the org keeps working with no
memory provider at all.

**Withdrawn.** The claim in ADR-0024 that Wadachi 0.14 ships `serve-http`. ROADMAP M8
slice 3 is to be rewritten to the stdio path.
