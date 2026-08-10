# ADR-0025: Provenance is Overmind's fact, and the browser is a view over a contract it does not own

- **Date:** 2026-08-09
- **Status:** accepted (implements the M8 memory UI; makes good on [ADR-0015](0015-agent-facing-memory-tools.md) decision 3)

## Context

M8's remaining acceptance criterion is *"a decision stored by an agent is
visible in the UI linked to the task that produced it."* Two things stand in
the way, and only one of them is the UI.

### The link does not exist

[ADR-0015](0015-agent-facing-memory-tools.md) already decided this, in decision
3:

> Writes stay orchestrator-authoritative; agents propose. The completion-time
> `store_memory` remains Overmind's, **with the task/issue as provenance** —
> that is what makes memory attributable and auditable.

That sentence has been true as an intention and false as a fact since M7.
`runner.rs` stores the task's title and description into the brain and keeps
nothing that points back at the task; `meeting.rs` stores a decision with the
topic in its rationale and the same silence. Nothing in the system can answer
"which task produced this?" — the exact shape of failure this project has now
caught four times (`permissions`, `model`, the cost ledger, the M8 blocker):
something written down, believed, and wired to nothing.

### The brain is not ours to shape

The obvious fix — add a `task_id` argument to `store_memory` — is not available.
[ADR-0003](0003-memory-via-mcp-wadachi-optional.md) makes the provider generic
and optional, and [ADR-0024](0024-managed-per-company-brain.md) just spent a
whole decision keeping a Wadachi-specific step out of the provisioning path. A
required field that only one implementation understands would undo that.

The reading side has the same shape from the other direction. Wadachi answers
`list_memories` with `{"memories": [...], "count": n}` and `list_decisions` with
`{"decisions": [...]}`, but that is Wadachi's shape, not the contract's. The
contract is three tool names and free-form text results.

## Decision

### 1. The link lives in Overmind, keyed by whatever the provider called it

A `memory_links` table records, for each memory Overmind stores: the company,
the provider's own identifier for it, and the **subject** that produced it —
`subject_type` + `subject_id`, `task` or `meeting`, the same polymorphic pair
`notifications` already uses.

The identifier comes from the tool result. Wadachi's `store_memory` answers with
`{"id": 1, "title": …, "filepath": …}` and `store_decision` with `{"id": 36, …}`,
so `store_memory` and `store_decision` in `mcp.rs` stop returning `()` and start
returning `Option<String>` — the id if one can be found, `None` otherwise. It is
stored as TEXT, not an integer, because the contract does not promise a number.

**Why in Overmind's database rather than the brain.** Which task produced a
memory is an orchestration fact, not a memory. Keeping it here means the link
survives swapping the provider, survives switching the company's brain off, and
is a SQL join away from the task's title, status and assignee — none of which
the brain knows. It also means the reverse lookup is an indexed query rather
than "list every memory and filter client-side", which is what a tag-only design
would force, since `list_memories` filters by project and category and nothing
else.

**A provider that returns no identifier gets no link, and that is fine.** The
memory is still stored; it simply shows up in the browser without a subject.
Memory has been best-effort since ADR-0003 and this is the same bargain.

### 2. The tag goes in anyway, because the vault outlives the app

Every memory Overmind stores also carries a `task:<id>` or `meeting:<id>` tag.
This is deliberately redundant: a Wadachi brain is a valid Obsidian vault, and a
memory whose provenance exists only in a SQLite file three directories away is a
memory that loses its provenance the moment someone opens the vault — which
ADR-0024 made an explicit selling point of the managed brain being a directory.

**Which one wins is stated, so the redundancy is not ambiguity.** The table is
authoritative for everything Overmind renders or queries. The tag is provenance
written into the artifact for whoever reads it outside Overmind. They cannot
drift in practice — one call path writes both, from the same two values — and if
a provider drops tags, the tag is simply absent, not wrong.

### 3. The browser tolerates a provider that does not fit

`Memory` gains `list_memories`, `list_decisions` and `recall`, and each parses
**defensively**: find an array under the expected key, take from each item only
the fields we recognize (`id`, `title`, `category`, `project`, `created_at`,
`content`), ignore everything else. A result we cannot parse is not an error —
it is a provider that does not expose a browsable list, and the UI says exactly
that instead of showing an empty page that looks like "you have no memories".

Two states that look identical to a careless implementation must not look
identical here:

| what is true | what the UI says |
|---|---|
| no provider configured | memory is not set up |
| this company's brain is switched off | this company's brain is off |
| provider present, nothing stored yet | no memories yet |
| provider present, unparseable answer | this provider cannot be browsed |

Search routes to `recall` when there is a query and `list_memories` when there
is not, because those are genuinely different operations — semantic search
versus enumeration — and pretending one is a filtered version of the other
would misrepresent both.

**The three shapes were run, not read.** Driving the real Wadachi server over
stdio and inspecting what comes back settled two things the source alone
implied wrongly. A memory's body arrives under a different key depending on the
call — `content` when stored, `rationale` for a decision, `preview` for a search
hit — so the normalizer accepts all three. And an *enumerated* memory carries no
body at all, only a `filepath`: a listed row showing nothing but its title is
correct, and would otherwise have looked like a bug worth chasing.

## Alternatives considered

- **A `task_id` argument on `store_memory`.** The clean modelling answer, and
  the one we would pick if we owned the brain. Rejected: it is a required field
  only Wadachi would understand, in the generic contract ADR-0003 and ADR-0024
  both work to keep generic.
- **Tag only, no table.** Cheap, portable, nothing new in the schema. Rejected
  on the reverse lookup: `list_memories` filters by project and category, so
  "the memories for this task" would mean fetching everything and filtering in
  Overmind — O(whole brain) per view — and the link would silently disappear
  for any provider without tags. Kept as the *secondary* representation, where
  those weaknesses do not matter.
- **Table only, no tag.** Tempting on single-source-of-truth grounds. Rejected
  because it makes the vault's portability a half-truth: ADR-0024 argued the
  brain being a plain directory you can open is part of the value, and a
  memory with no trace of where it came from is worth much less on that reading.
- **Mirroring memories into Overmind's own tables** so the UI never depends on
  the provider being up. Rejected outright: two copies of the corpus, a sync
  problem, and it re-creates inside Overmind the thing ADR-0004 refused to
  vendor. The UI depending on the brain being reachable is correct — that *is*
  the integration.
- **Making the memory browser a task-detail panel only**, with no standalone
  view. Smaller, and it serves the acceptance criterion literally. Rejected: the
  organization's memory is a thing you want to read on its own ("what does this
  company know?"), and hiding it inside a task makes it discoverable only by
  someone who already knows it exists.

## Consequences

- **`store_memory` and `store_decision` change signature** (`()` →
  `Option<String>`). Both callers already ignored failures; they now ignore a
  `None` the same way.
- **Old memories have no links.** Everything stored before this lands shows up
  in the browser with no subject, permanently — there is nothing to backfill
  from, since the brain never recorded the task. Stated rather than papered
  over.
- **The browser is only as good as the provider.** With Wadachi it lists,
  searches and shows categories; with a minimal conforming server it may show
  nothing browsable at all, and says so. That is the cost of the contract being
  generic, and it is a cost ADR-0003 already accepted on purpose.
- **Deleting a task does not delete its links.** Same reasoning as ADR-0024
  declining to delete a company's brain: dropping recorded provenance should be
  a decision someone makes, not a side effect. The browser tolerates a link
  whose subject has gone.
- **This does not give agents memory tools.** ADR-0015's ceiling — agents
  calling `recall` mid-task over a per-company HTTP endpoint — remains M8 slice
  3 and is untouched here.
