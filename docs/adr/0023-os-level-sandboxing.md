# ADR-0023: The agent runs in a cage, denied by default

- **Date:** 2026-08-07
- **Status:** accepted

## Context

Since [ADR-0005](0005-structured-agent-characterization.md) the promise has been
that what an agent may do is *enforced server-side, not suggested via prompt*.
M14 made half of that true — `task:code` / `task:knowledge` are refused at
checkout — and was explicit about the other half:

> everything else (`repo:read`, `pr:approve`, …) — *declared*, compiled into the
> prompt so the agent knows its remit, but not policed: we shell out to an
> external CLI and cannot stop it. Real enforcement of those needs sandboxing
> (M10) — pretending otherwise would be the "security by prayer" ADR-0005
> rejects.

That was honest, and the honesty has been getting more expensive. M17 gave
agents arbitrary file I/O; M14 gave them domains that imply browsing. Today an
agent runs as the user, in a `sh -c`, with the user's whole machine reachable:
`~/.ssh`, `~/.aws`, the browser profile, and — pointedly — Overmind's own source
and its `overmind.sqlite`, audit chain included.

**The threat model is not an external attacker.** Overmind runs on one person's
machine, for that person; anyone who has the machine can run the CLI directly.
The realistic failures are an agent that misreads its task, and a **prompt
injection** arriving inside material the user handed it — a PDF, a scraped page,
a repository file (M17 made that surface much larger). Both are accidents of
capability, and capability is what a sandbox removes.

### The acceptance criterion is three mechanisms, not one

M10's criterion — *a deliberately malicious task ("read `~/.ssh`, push to main,
exceed budget") fails at every layer* — decomposes into three unrelated
defences, and only one of them is this ADR:

| | mechanism | state |
|---|---|---|
| read `~/.ssh` | the sandbox | **this ADR** |
| exceed budget | budget gate | done, M6 + [ADR-0022](0022-conversational-spend-under-budget.md) |
| push to main | git credential isolation | **not the sandbox** — see below |

The sandbox cannot stop a push, because it cannot close the network: the agent's
whole job is to reach the Claude API. Anything reachable over an open socket
with the user's ambient credentials stays reachable. That is a separate slice.

## Decision

Every spawn of agent-controlled work is wrapped in `sandbox-exec` with a
**deny-by-default** profile. Two sites: `runner.rs` (task runs) and `ceo.rs`
(chat and meeting turns). The other two spawns in the codebase are ours and stay
outside — `git`, invoked by us with arguments we construct, and the MCP memory
server, which is configuration the user wrote.

The profile imports the system's `bsd.sb` base, then denies everything and
allows back:

- **read** on the system paths a process needs to exist at all (`/usr`, `/bin`,
  `/System`, `/Library`, `/private/etc`, `/private/var`, `/dev`);
- **read and write** on the run's own directory — the worktree for a `code` task,
  the scratch dir for a `knowledge` task or a conversational turn — and on the
  temp directories a compiler or a package manager expects;
- **read** on the adapter's install directory and **read/write** on its state
  directory, both configurable, because where a vendor CLI lives is an
  installation fact and not ours to hardcode;
- **network**, in full, for the reason above.

Everything else — the home directory, other volumes, Overmind's own data dir —
is denied.

### Why deny-by-default rather than a list of forbidden places

An allow-by-default profile with targeted denials (`~/.ssh`, `~/.aws`, the
keychain) would work with any adapter and never break on a vendor update. It is
also a blocklist, and a blocklist protects exactly the places someone thought
of. `~/.config/gh`, `~/Documents`, the next credential file a tool invents:
all open. ADR-0005 rejected prompt-only characterization as "security by prayer";
a blocklist is the same prayer with a shorter list.

Deny-by-default fails in the opposite direction, and the direction matters: when
the profile is wrong the agent **does not start**, loudly, instead of quietly
having more reach than intended.

### Escape hatches, because the profile will be wrong sometimes

`OVERMIND_SANDBOX_ALLOW` adds paths (colon-separated) for a setup this profile
does not anticipate; `OVERMIND_SANDBOX=off` disables it entirely. Both exist
because a security control nobody can adjust is a security control people
disable by deleting, and the second is what makes the first honest: if you are
going to turn it off, turn it off deliberately and visibly rather than by
editing a profile until it permits everything.

## Alternatives considered

- **Allow-by-default with targeted denials.** Rejected above: it is a blocklist,
  and the milestone exists to stop pretending.
- **Leased sandboxes from a provider**, which is what Paperclip does —
  `environments` with a `driver`, `environment_leases` carrying a
  `provider_lease_id`. Studied per the fidelity rule and rejected on shape:
  their model is cloud-oriented and multi-tenant, ours is one person's laptop.
  Their `sandbox-exec` usage is confined to a single test file. We adopt the
  *vocabulary* where it fits and not the mechanism (recorded in
  [PAPERCLIP-ALIGNMENT.md](../PAPERCLIP-ALIGNMENT.md)).
- **Containers (Docker) per run.** Strong isolation, and Overmind already ships
  a Dockerfile. Rejected for the local case: it moves the agent away from the
  user's toolchain — the compilers, language servers and credentials a coding
  task legitimately needs — and turns a `sh -c` into an image-management
  problem. It remains the right answer for a hosted Overmind, which is not this.
- **Doing credential isolation first** and sandboxing later. Tempting, since
  "push to main" is the scarier line in the acceptance criterion. Rejected on
  size of exposure: arbitrary read of the whole filesystem is the larger hole,
  and it is the one an injected prompt reaches first.

## Consequences

- **Two parameters are installation-specific**: where the adapter binary lives
  and where it keeps its state. Defaults suit a standard Claude Code install;
  `OVERMIND_SANDBOX_ALLOW` covers the rest.
- **A vendor update can break the profile**, and it will break it loudly — the
  agent fails to start. That is the failure mode we chose.
- **`sandbox-exec` is deprecated by Apple** yet present and functional on
  current macOS (verified on 26.5). When it goes, the replacement is the App
  Sandbox entitlement model or a container; the wrapper is one module, so the
  blast radius of that migration is one module.
- **macOS only.** On other platforms the wrapper is a no-op and says so at
  startup rather than pretending to protect. Linux support is already in the
  icebox; `bubblewrap` is the natural counterpart when it arrives.
- **Declared permissions are still declared.** This ADR removes *capability*,
  not the gap between `repo:write` and what the sandbox knows about git. An
  agent with the worktree writable can still commit to it; what it can no longer
  do is read your keys or touch anything outside its own run.
- **The network stays open**, so anything reachable with ambient credentials
  stays reachable. Credential isolation is the next slice, and until it lands
  the third line of M10's acceptance criterion is not met.
