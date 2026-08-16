# ADR-0029: The cage inside the container — an unprivileged uid, and Landlock where the kernel has it

- **Date:** 2026-08-16
- **Status:** accepted

## Context

[ADR-0023](0023-os-level-sandboxing.md) gave macOS a deny-by-default cage and
was explicit that it was macOS only, naming `bubblewrap` as "the natural
counterpart when Linux support arrives". Its closing addendum stated the
consequence plainly: **the permission flag rides on the cage, and only on the
cage.** No cage, no `--dangerously-skip-permissions` (D34/D35), and in headless
`-p` mode the CLI then denies every write it attempts.

M19 is where that consequence stops being theoretical. The container is how
everyone who is not on macOS is meant to have Overmind — and how Windows is
supported at all — and `docker compose up` has never produced a working agent
run. `sandbox::available()` answers `cfg!(target_os = "macos")`, so inside a
Debian image there is no cage, no flag, and no writes.

The roadmap named two candidates, `bubblewrap` and Landlock, and assumed one of
them would do. **Both were measured before anything was written, and neither
works on both platforms that matter.**

### What was measured (2026-08-16)

| | Docker Desktop — how Mac and Windows have Overmind | Docker on Linux, default settings |
|---|---|---|
| **Landlock** | ❌ the kernel does not have it | ✅ |
| **bubblewrap** | ✅ | ❌ no user namespace |

Almost exactly complementary, in the least useful way.

**Landlock is absent from Docker Desktop.** Kernel `6.10.14-linuxkit`, and its
own configuration says so — `# CONFIG_SECURITY_LANDLOCK is not set`, with
`CONFIG_LSM="yama,loadpin,safesetid,integrity,bpf"`. Syscalls 444/445/446
return `ENOSYS`, identical to the nonexistent syscall 600 used as a control,
while `getpid` and `faccessat2` answer normally — so the probe is sound and the
syscalls genuinely are not there. This is not seccomp filtering them: Docker
Desktop already runs `seccomp,profile=unconfined`. It is a known LinuxKit
limitation, not a fact about one machine.

**bubblewrap is blocked by Docker on Linux.** The default seccomp profile
permits `clone`/`clone3`/`unshare`/`setns` **only with `CAP_SYS_ADMIN`**; the
rule that applies without it is a masked comparison against `0x7E020000` —
exactly the union of the `CLONE_NEW*` flags — requiring them all to be zero.
Reproduced by passing that profile explicitly: `bwrap: No permissions to create
new namespace`. With `--cap-add SYS_ADMIN` it gets further and dies on
`bwrap: pivot_root: Operation not permitted`.

So bubblewrap would ask the user to **weaken their container in order to gain a
security feature**. That is backwards, and it is the whole argument against it.

(A web search claimed the opposite — that Docker's default profile blocks the
Landlock syscalls. It is false: all three `landlock_*` names are in the default
allowlist. The profile moved to the `moby/profiles` repository, and the old
path in `moby/moby` now 404s, which is likely how the claim survived.)

### The defect nobody had counted

While measuring, a fourth defect appeared, independent of all three the
milestone opens with. The `Dockerfile` has no `USER` directive, so the server
and its agents run as **root** — and the CLI refuses the flag outright:

```
$ docker run --rm cli-probe sh -c 'claude -p "say hi" --dangerously-skip-permissions …'
--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons
EXIT=1

$ docker run --rm --user 10001:10001 -e HOME=/home/agent cli-probe sh -c '…same…'
{"is_error":true, …, "result":"Not logged in · Please run /login", …}
```

As root the flag is refused before authentication is even attempted; as an
unprivileged uid it is accepted and the run reaches the API, stopping only for
want of credentials. Measured with claude 2.1.233.

**This is what settles the decision.** Had we implemented Landlock perfectly,
`docker compose up` would still have failed, and the cage would have looked like
the cause. Whatever cage we choose, the agent must not be root — so an
unprivileged uid is not one option among several. It is a prerequisite, and it
is also, on its own, a real boundary.

## Decision

There is no single mechanism. There is a **set of mechanisms, chosen by what the
platform actually offers**, and one predicate that says whether a real boundary
was obtained.

### 1. In the image, the agent is never root

The server is the privileged party — PID 1, root — and drops to an unprivileged
`agent` uid for every spawn of agent-controlled work. `tokio::process::Command`
exposes `uid()` and `gid()`, so this needs no `unsafe`, and it goes in
`sandbox::command()`, which is already the single spawn site for both callers
(`runner.rs`, `ceo.rs`).

What that boundary is made of:

- **Overmind's own shelves are `0700` and stay the server's**: every company's
  brain, the collected artifacts, the files people attached. `overmind.sqlite`
  is `0600`, and so are its WAL and shared-memory files — those carry the same
  rows before a checkpoint, so leaving them open would make the main file's mode
  decorative.
- **What merely holds run directories is `0711`** — traversable, not listable.
  An agent enters its own run by the exact path it was handed and cannot
  enumerate its siblings. The data directory itself is `0755`: it holds these
  names and nothing else, and the agent has to walk through it to reach its own
  work. (An earlier draft of this ADR said `0700` throughout, which would have
  denied the agent the path to its own run directory — the same defect, in the
  same place, that emptied the macOS cage on 2026-08-13.)
- **The run's own directory** — the worktree for a `code` task, the scratch dir
  for a `knowledge` task or a conversational turn — is handed to the agent uid,
  and the one MCP token file that run needs is handed over **by file**, never by
  opening the directory that holds every run's token.
- **The agent has its own `HOME`**, which is where its CLI keeps credentials.

The layout is applied by the server at startup rather than baked into the image,
because `/data` is a volume: it outlives any one build, and a boundary that only
protects installations created after it shipped is one nobody can reason about.
It is a no-op wherever no agent uid is configured — quietly re-permissioning
somebody's own directory is not ours to do.

One implementation note worth keeping, because it is load-bearing and invisible:
setting `uid` on the command is enough. When a uid is set and no explicit group
list was given, the standard library calls `setgroups(0, NULL)` **before**
`setuid`, so the child does not inherit the server's supplementary groups. That
same ordering rules out doing any of this in `pre_exec`: those closures run
*after* the uid change, when the process is no longer privileged enough to make
it.

This is the classic Unix answer and it needs no kernel feature, no capability
and no change to seccomp. It therefore works identically on Docker Desktop and
on Docker for Linux — which is the property the other two candidates could not
supply.

**This mechanism belongs to the image, not to Linux.** On a user's own machine
Overmind runs as that user, and inventing a second uid on someone's laptop is
not ours to do. Availability is therefore asked as a question about the running
process — can I change uid at all, and is there a distinct uid to change to —
rather than about the operating system.

### 2. Landlock where the kernel offers it, as a second layer

Where Landlock exists — Linux natively, our CI, Docker on a real Linux host, but
not Docker Desktop — it is applied **in addition**, and it buys the one thing
the uid boundary does not: confinement to the run's own directory, which is what
macOS has had since ADR-0023.

The ruleset is created and populated **in the parent**, and the child, inside
`pre_exec`, does only two things: `prctl(PR_SET_NO_NEW_PRIVS, 1)` and
`landlock_restrict_self(fd)`. Both are bare syscalls. This split is not
stylistic — `pre_exec` runs after `fork` in a multithreaded process, where
anything that allocates can deadlock on a lock another thread held at fork time,
and every ergonomic Landlock wrapper allocates.

**Best-effort is refused.** If the kernel advertises Landlock and the ruleset
cannot be applied, that is a failure, not a quiet downgrade to less protection
than we claimed. The failure mode ADR-0023 chose — the agent does not start,
loudly — is the one that applies.

### 3. `available()` stops being a boolean

`pub fn available() -> bool` cannot express any of the above. It becomes a
question of **which mechanism, if any**, and `caged()` stays exactly what it was:
one predicate, asked by both the spawn and the command builder, true only when a
real boundary was obtained. It is what grants the permission flag, and
`the_permission_flag_never_travels_without_the_cage` continues to hold it.

| where | mechanism | caged |
|---|---|---|
| macOS | `sandbox-exec` (ADR-0023) | ✅ |
| the image, on Docker Desktop | unprivileged uid | ✅ |
| the image, on Docker for Linux | unprivileged uid **+** Landlock | ✅ |
| Linux natively, as the user | Landlock, if the kernel has it | ✅ / ❌ |
| anything else | — | ❌ |

The last row keeps ADR-0023's honest failure direction: no cage means read-only
agents that analyse and propose but do not write, and say so.

### 4. Forced by the above: where the agent's credentials live

An unprivileged agent with its own `HOME` needs its CLI's credentials to be
somewhere that survives a rebuild, which is the milestone's other open item
arriving through the front door rather than as scope creep. The image ships an
agent CLI and keeps `~/.claude` for the agent uid on a named volume;
`ANTHROPIC_API_KEY` is passed through when set. A credential baked into a layer
would be a credential in the image, and one living only in the container is one
lost on the next `docker compose up --build`.

The volume is mounted **by default**, not offered as a commented-out line: "your
sign-in survives a rebuild" is not a property anyone should have to discover.
And because a fresh named volume arrives owned by root and the agent is not
root, the server takes ownership of the agent's home at startup — the same place
and the same reason as the data directory's layout. Without it a `claude login`
succeeds and then cannot write what it just obtained, and the symptom is a
sign-in that never sticks: a failure a long way from its cause, which is the
shape of defect this milestone keeps finding.

The same startup step is what makes `OVERMIND_AGENT_UID` a real escape hatch
rather than a setting that half-works. On Linux a mounted repository keeps its
host ownership, so an agent on uid 10001 may not be able to write it — and a
`code` task needs to, because a worktree's git metadata lives in the repository
rather than in the run directory. Pointing the uid at your own `id -u` fixes
that, and the boundary survives it: what this ADR requires is that the agent is
not the *server*, and any unprivileged uid holds that.

## Alternatives considered

- **bubblewrap.** The roadmap's first candidate, and ADR-0023's own guess.
  Rejected on measurement: blocked by Docker's default seccomp profile, and
  recoverable only by asking every user to run with `--cap-add SYS_ADMIN` or
  `seccomp=unconfined` — weakening the container to gain a security feature.
  Even then it failed at `pivot_root`.
- **Landlock alone.** Keeps ADR-0023's deny-by-default promise in its fullest
  form, and loses on audience: Docker Desktop is how Mac and Windows users get
  Overmind, and there it does not exist. M19 would not close.
- **The container as the cage.** Rejected in ADR-0023 and still wrong, for the
  reason the roadmap states: a container isolates itself from the *host*, while
  the cage isolates the *agent* from *Overmind*. Inside the image the agent sits
  next to `overmind.sqlite`, every company's brain, and the token files. The
  adversary is already inside.
- **A uid per run**, rather than one `agent` uid. It would give run-to-run
  isolation without Landlock, on every platform. Rejected for now: it multiplies
  the credential problem — every allocated uid needs to read `~/.claude` — and
  buys, at much greater cost, what Landlock already provides where it exists.
  Reconsider if run-to-run isolation is ever wanted on Docker Desktop
  specifically.
- **`CAP_SETUID` on the binary instead of a root PID 1.** Would keep the server
  unprivileged, which is tidier. Rejected as a refinement rather than a
  decision: file capabilities are one more thing to preserve across a build, and
  the boundary that matters — agent below server — is identical either way.
- **Doing nothing off macOS and documenting it.** The status quo. It is what
  makes the container ship a product that cannot work, silently, which is the
  defect M19 exists to end.

## Consequences

- **The image gains a second user**, and the server must run privileged enough
  to drop to it. A `docker compose` invocation that overrides `user:` will take
  that ability away, and the honest result is an uncaged run — which now reports
  itself rather than proceeding.
- **Two mechanisms mean two things to test.** The paired-probe discipline M10
  used stays: every denial is proven against the identical run without the cage,
  because a denial only proves something if the same script succeeds without it.
  Landlock's probes can only run where Landlock exists, which is why CI runs on
  both platforms since `fee4835`.
- **Docker Desktop gets a weaker cage than Docker on Linux**, and the difference
  is precisely run-to-run confinement: one agent can read another run's
  worktree. Both keep the agent out of Overmind's own data, which is the
  boundary the threat model is about. This is written into
  [THREAT-MODEL.md](../THREAT-MODEL.md) as a difference, not smoothed over.
- **If LinuxKit ever enables `CONFIG_SECURITY_LANDLOCK`**, Docker Desktop gains
  the second layer with no change here — the mechanism is chosen by asking the
  kernel, not by naming the platform.
- **`OVERMIND_SANDBOX=off` still turns everything off**, deliberately and
  visibly, for the same reason ADR-0023 gave: a control nobody can adjust is a
  control people disable by deleting.
- **Git credential isolation is untouched.** It is environment-based and
  orthogonal to which cage holds the process.
- **The permission flag's rule is unchanged and now has more ways to be earned.**
  That is the point of keeping one predicate: an uncaged agent with permissions
  skipped remains the one combination that cannot occur, however many mechanisms
  the set grows to.

## Measured, not asserted (2026-08-16)

The uid half is in, and held by the pair M10's discipline calls for — the same
probe run caged and uncaged, asserted against each other, because a denial only
proves something if the identical run succeeds without it. `docker compose`'s
own image, a knowledge task through the API, and the probe reporting what it can
reach from inside the run:

| | uid | `overmind.sqlite` | every company's brain |
|---|---|---|---|
| caged (default) | 10001 | DENIED | DENIED |
| `OVERMIND_SANDBOX=off` | 0 | READABLE | READABLE |

Both runs delivered the file, so the boundary is not "the agent could not do
anything"; the audit chain verified in both. The server announces which
mechanism it got at startup — `agent confinement: unprivileged uid 10001` — for
the reason ADR-0023 gave and this milestone proved twice: a cage nobody can see
the absence of is how "no run had ever changed a file" went unnoticed for a
month.

Writing that test found one thing in passing that no unit test would have: the
stub adapter is mounted from a `mktemp -d`, which is `0700`, and an agent that is
no longer root cannot traverse it. The adapter failed to *start*, which looks
exactly like the boundary working and is nothing of the kind. Anything mounted
for an agent now has to be reachable by a uid that did not create it — a general
consequence of this ADR, and the first of what will be several.
