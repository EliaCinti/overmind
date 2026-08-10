# ADR-0024: Every company gets its own brain, and provisioning it is making a directory

- **Date:** 2026-08-09
- **Status:** accepted (implements [ADR-0004](0004-wadachi-first-party-managed-brain.md) point 1)

## Context

M7 shipped the memory contract: an MCP client, `get_context` on start,
`store_memory` on finish, best-effort everywhere. What it did **not** ship is
isolation. There is exactly one brain — whatever `OVERMIND_MEMORY_CMD` points
at — and every company on the server writes into it.

Companies are told apart inside that one brain by an argument: `store_memory`
passes `company_id` as the `project` field. That is a convention, not a
boundary. `get_context` derives the project from a `cwd` that is a worktree path
under Overmind's data dir, so it is not obviously anyone's project; and a
`recall` without a project scope reads the whole brain by design. Two companies
on one server can see each other's memories, and nothing in the system says
they should not.

The sharper problem is whose brain it is. If a user points
`OVERMIND_MEMORY_CMD` at their own Wadachi — the obvious thing to do, and what
the M7 acceptance run did against a throwaway brain — then agents write into a
personal brain. [ADR-0004](0004-wadachi-first-party-managed-brain.md) forbids
exactly that ("Overmind never reads or writes a user's personal Wadachi
brain"), and so does rule 6 of `CLAUDE.md`. Today only care enforces it.

### The dependency ADR-0004 declared was worked on — and is not finished

ADR-0004 and `ROADMAP.md` both said M8 waits on Wadachi supporting concurrent
multi-agent access. Work for it landed in **Wadachi 0.14.0 on 2026-07-20**:
SQLite in WAL with `busy_timeout` on every connection, `index.md` rewritten via
temp file + `os.replace`, memory files created `O_CREAT|O_EXCL` so two
concurrent writes of the same title get distinct files instead of one silently
winning — with three concurrency tests, including 24 parallel writers. The
commit names "più agenti di Overmind in parallelo" as the motivating case.

The roadmap line outlived *that* by three weeks and is corrected alongside this
ADR. But the first version of this section said the dependency was **satisfied**,
on the strength of a changelog entry, and that was wrong: see the addendum
below, which is the more important half of this ADR.

## Decision

**A company's brain is a directory, and provisioning it is creating that
directory.**

Each company gets `<data-dir>/companies/<company-id>/brain/`. Before the first
memory call for a company, Overmind creates the directory; every memory call
made on that company's behalf spawns its memory server with `BRAIN_DIR` set to
it. That is the whole mechanism.

There is no init step, no template, no schema to lay down. A memory server
pointed at an empty directory builds whatever it needs on first connection —
Wadachi's `MemoryStore.__init__` does `mkdir(parents=True, exist_ok=True)` and
creates its own layout. "One click and a company has a brain" turns out to cost
one `create_dir_all`.

**Measured against the real server, not assumed.** The whole decision rests on
"a memory server pointed at an empty directory does the rest", so that was
driven directly: the real Wadachi MCP server spawned over stdio exactly the way
`mcp.rs` spawns it, with `BRAIN_DIR` set to a throwaway path. It created
`brain.db`, `index.md`, `global/` and `projects/` there, stored a memory and
recalled it — and the memory came back as **`"id": 1`**, which is the load-bearing
detail: a brain that numbers its first memory 1 is a new brain, not the personal
one with 180-odd in it. The personal brain's mtime did not move.

**Overmind never runs a Wadachi command.** This is the part worth defending. It
would be easy — and it would work — to shell out to `wadachi init --brain-dir
…`. It would also make ADR-0003's generic `MemoryProvider` a fiction: the code
path that gives a company a brain would only work for one implementation. What
Overmind sets instead is an environment variable, which any server is free to
honour or ignore.

### Three modes, and the default changes

| `OVERMIND_MANAGED_BRAIN` | behaviour |
|---|---|
| unset / `on` (**default**) | per-company brain under the data dir |
| `off` | today's behaviour — one shared brain, wherever the memory command points |

Plus a per-company switch, `brain_enabled`, on the `companies` row. Off means
every memory call for that company is a no-op — the same no-op as having no
provider configured, which is a path M7 already tests.

The default flips: a fresh install now gets isolated managed brains rather than
one shared one. That is the point of the milestone, and the escape hatch is
there for the user who deliberately wants their agents in a brain they chose.

### What this does not promise

`BRAIN_DIR` is Wadachi's convention. A conforming MCP server that ignores it
gets one shared brain and no isolation — degraded, not broken, and silent. We
accept that rather than invent a handshake ("do you support per-brain
routing?") that no other server implements and no user has asked for. It is
written down here and in `THREAT-MODEL.md` so the limit is a known one.

Isolation is also **not a security boundary**. It is separation between
organizations that trust the same operator, on one person's machine, enforced
by a path. An agent cannot reach another company's brain because
[the cage](0023-os-level-sandboxing.md) denies it the whole data dir — but that
is the sandbox's doing, not this ADR's.

## Alternatives considered

- **Keep one brain; isolate with the `project` argument.** Free, no migration,
  and the argument is already being passed. Rejected: a scope you ask for
  politely is not a boundary. `recall` without a project reads everything, and
  the memory UI in the next slice would have to be trusted to always pass the
  filter. The brain also stays the user's, which is the thing ADR-0004 forbids.
- **Run `wadachi init` per company.** The obvious provisioning step, and it does
  more than `mkdir` (guided config, MCP wiring). Rejected: it is a
  Wadachi-specific command in the one code path that most needs to stay generic,
  and everything it would buy us here, the server does for itself on first use.
- **One long-lived server process per company, supervised by Overmind** — the
  fuller reading of ADR-0004's "launch and supervise", and now practical since
  Wadachi 0.14 ships `wadachi serve-http`. Rejected for this slice, not
  forever: the stdio pool from M7 already gets connection reuse and per-call
  isolation for free, while a supervised daemon adds lifecycle, ports, tokens
  and a new class of failure (the process died; the port is taken) before
  anything asks for it. Revisit if pool churn shows up as latency.
- **A brain per project rather than per company.** Finer-grained, and it maps to
  Wadachi's own `project` notion. Rejected: organizational memory that does not
  cross projects is most of the value gone — the CEO's context and a builder's
  hard-won lesson belong to the company.

## Consequences

- **Existing installs change behaviour on upgrade.** Companies that were writing
  into a shared brain start writing into fresh empty ones. Nothing is lost or
  moved; the old brain is untouched and reachable with
  `OVERMIND_MANAGED_BRAIN=off`. No migration is offered because a memory is
  cheap to re-earn and a wrong-brain import is not.
- **A cache of `Memory` handles per company** now lives in `AppState`.
  `with_brain_dir` builds a fresh connection pool each time it is called, so
  calling it per task would respawn servers forever; handles are created once
  per company and reused.
- **The data dir grows a brain per company.** Deleting a company does not yet
  delete its brain — deliberately, since dropping a company's memory should be
  a decision someone makes, not a side effect. It is a gap, and it is listed as
  one.
- **`BRAIN_DIR` inside `OVERMIND_MEMORY_CMD` silently defeats this**, and the
  README used to recommend exactly that: `OVERMIND_MEMORY_CMD="BRAIN_DIR=/path
  wadachi"`. The command runs through `sh -c`, so an inline assignment wins
  over the environment we set, and every company quietly shares one brain. No
  code can beat a user's own shell assignment, so this is documented rather
  than defended — in the README, next to the flag (`OVERMIND_MANAGED_BRAIN=off`)
  that expresses the same intent visibly. Both the README and
  `docker-compose.yml` were corrected with this ADR.
- **Memory is now company-scoped in a way the UI can rely on**, which is what
  makes the next slice — browsing an org's memories, decisions linked to the
  tasks that produced them — a matter of reading one brain rather than
  filtering a shared one.

## Addendum — the concurrency guarantee is real but incomplete (2026-08-09)

This ADR originally called ADR-0004's dependency *satisfied*, citing Wadachi's
0.14.0 changelog. Elia pushed back — the requirement was several agents writing
one brain **at the same time**, and had that actually been demonstrated? It had
not. So it was measured, and the answer is no.

**The test.** Eight separate `wadachi.server` processes, all pointed at one
`BRAIN_DIR` — which is exactly what Overmind's stdio pool does on the managed
path — released from a barrier so they collide, five `store_memory` calls each.
Forty writes expected.

**The result, over five runs: 1, 3, 2, 1 and 0 memories lost.** Each loss is
`Error executing tool store_memory: database is locked`, returned in ~50ms
rather than after the 30-second `busy_timeout`. Four runs in five lost at least
one memory.

Worse than losing it: `store_memory` writes the markdown file *before* the row,
so a failed write leaves **40 files on disk and 37 rows in the database**. The
memory exists in the vault and is invisible to `list_memories`. A clean loss
would have been better.

**The cause is one line, and it is the line whose comment claims safety.**
`MemoryStore._conn` runs `PRAGMA journal_mode=WAL` on every connection, above
the comment *"re-applying it per connection is idempotent and cheap"*. It is
idempotent. It is not lock-free: setting the journal mode needs a brief
exclusive lock, and SQLite does **not** invoke the busy handler for it — so it
returns `SQLITE_BUSY` immediately while any other connection is mid-write. The
timeout that the rest of the design leans on never gets a chance to apply.

Demonstrated rather than reasoned: the same eight-process test against a server
with that single pragma removed and nothing else changed passes 40/40, twice,
with the slowest write at 0.07s. The fix is to set the journal mode once, where
the database is created, not on every connection.

**What this does and does not change here.** Slices 1 and 2 stand: per-company
routing, provisioning and browsing are unaffected, and memory has been
best-effort since ADR-0003, so a lost write is logged and swallowed rather than
failing a task. What it changes is the confidence. Under Overmind's default pool
of four the collision window is narrower than this test's, but it is not zero,
and "the brain is concurrency-safe" is not a sentence this project has earned
yet.

**The fix belongs in the Wadachi repo** (rule 6 of `CLAUDE.md`: changes Wadachi
needs are developed there, never vendored here), and that is where it was done
— Wadachi 0.14.1, [PR #2](https://github.com/EliaCinti/wadachi/pull/2). Chasing
it down found a **second and worse defect** than the one this addendum started
with: `run_migrations` was not serialized across processes, so several agents
opening a *freshly provisioned* brain at the same moment made all but one die
with `UNIQUE constraint failed: schema_version.version`. That loser does not
lose a memory — it fails to open the brain, on the first task a new company
ever runs.

0.14.1 serializes migration with a file lock, sets `journal_mode` once instead
of per connection, opens every write transaction `BEGIN IMMEDIATE`, and deletes
the markdown file when its insert fails so the vault and the index cannot
disagree. It also adds the two multi-process tests that were missing — the old
24-writer test used **threads in one process**, which is not the shape Overmind
creates and is why a real defect lived under a green suite.

Worth recording honestly: the two defects mask each other, and applying either
fix alone made the reproduction stop failing, so which one caused which loss was
never isolated. The migration race is the one with a deterministic
reproduction.

ADR-0015's agent-facing ceiling — N agents each calling memory tens of times per
task — would have hit both far harder than the orchestrator's two calls do, so
this is a prerequisite of M8 slice 3 rather than a tidy-up.

**The habit worth keeping.** A changelog is a claim, not evidence. This ADR
already knew that where it mattered — the `BRAIN_DIR` behaviour was driven
against the real server instead of read from the source — and then took the
concurrency guarantee on trust because it was written down convincingly. The
rule that catches this is the project's own: run the thing.
