# Live smoke run

> The manual follow-up M2 has been carrying since 19 July 2026: everything from
> M2 to M18 was accepted against a **stub adapter**. This is the checklist for
> exercising the same paths against the real Claude Code CLI.
>
> **Discharged 2026-08-09.** Every section below has been run against the real
> CLI; the results are in "What the run found". Keep the checklist — it is the
> script to re-run whenever the adapter, the sandbox or the prompt layer changes.

## Why a stub cannot answer these

A stub is a shell script. It does not authenticate, spawn subprocesses, load
dylibs, write its own state, call the network, or emit the envelope the real CLI
emits. Every question below turns on one of those. The first pass already found
two ledger defects that every test had passed over, because the stubs emitted
the envelope we imagined rather than the one that exists.

## What the run found (2026-08-07 → 09, ~$7)

**Five defects, every one of them invisible to the stubs**, and four of the five
are the same shape: a thing believed, documented, and wired to nothing.

1. **No cost was attributed to a model.** The real envelope has no top-level
   `model`; everything was filed as `unknown`. Fixed by reading `modelUsage`.
2. **Sub-cent runs rounded to zero** — invisible spend. Now floored at 1¢.
3. **The plan layer was dead.** The real CLI returns one envelope with the plan
   nested in `.result`; the parser scanned raw lines. No plan had ever been
   parsed live: no tasks, no proposals, the envelope itself shown as the reply.
   M12 / M12.5 / M13 / M15 were inert against the real adapter.
4. **Tasks an agent opened could never run.** `goal_id` was bound `NULL`, and a
   `code` run needs `task → goal → project → workspace` to find a repo. Every
   task the CEO opened from a chat or a meeting was born unable to start. See
   the addendum to [ADR-0008](adr/0008-execution-sessions-and-atomic-checkout.md).
5. **No `code` task could ever change a file.** The CLI's permission system
   assumes a person at a terminal; headless, every `Edit`, `Write` and `Bash` is
   denied. The agent read, diagnosed, wrote the patch into its prose — and
   changed nothing. M2's central criterion had only ever been met by stub shell
   scripts, which write freely because they are shell scripts. See the addendum
   to [ADR-0023](adr/0023-os-level-sandboxing.md): the flag that fixes it rides
   on the cage, and only on the cage.

Two smaller ones, both the same instinct — show the person what the agent said,
not the adapter's envelope: a failed run reported `agent exited with code 1`
while the envelope said `Credit balance is too low`; a finished run showed the
raw envelope where its summary belonged.

### And then it worked

Re-run after the fixes, end to end:

- **§1 knowledge, §4 chat** — reply in role, spend against the right agent,
  reservation released, model recorded correctly. The CEO proposed a team and
  hired nobody until accepted.
- **§2 code** — worktree and branch created, git working in the cage,
  `permission_denials: []`, the bug fixed (`a - b` → `a + b`) with the two
  regression tests the CEO had asked for, the diff back in the drawer, the
  written report collected from `deliverables/` and kept out of the diff.
- **§3 files** — the CSV reached the agent and it quoted the real totals
  (Nord 87.800 €, total 178.450 €, checked against the file). It also caught
  that the task said "Q1–Q2" while the data stopped at April, and refused to
  invent the missing months. Outputs came back typed: `.md` inline, a real
  104 KB `.png` chart served from disk, `.py` as text.
- **§5 meeting** — nothing ran before approval; the room **paused** on Sasha's
  cap with `Sasha reached its monthly budget: €2.27 of €2.47 spent`, resumed
  after the cap was raised, and reached a specific decision in three turns.
  Cost recorded per turn, per turn. ADR-0022's headline behaviour, live.
- **§6 audit** — valid over all 56 events, including `budget.blocked`,
  `meeting.paused` and `meeting.decided`.

### Worth knowing next time

- A `.pyc` landed in the diff. Ignoring build output is the repository's job —
  the toy repo has no `.gitignore` — but a real project will want one.
- On a platform without the cage, agents are read-only by design. That is the
  consequence recorded in ADR-0023's addendum, not a bug to be surprised by.

## Setup

Use a **throwaway database and data dir** — a smoke run should not land in the
working company.

```sh
cd /Volumes/ExtremeSSD/overmind
cargo build --release            # or run the debug binary
(cd web && npm run build)        # the server serves web/dist at /

export OVERMIND_DB=sqlite:///tmp/overmind-smoke/smoke.sqlite
export OVERMIND_DATA_DIR=/tmp/overmind-smoke/data
export OVERMIND_ADDR=127.0.0.1:7071
mkdir -p /tmp/overmind-smoke/data
cargo run --release
```

Leave `OVERMIND_AGENT_CMD` **unset** — that is the whole point; the default is
the real CLI. Leave `OVERMIND_SANDBOX` unset too: on is the default.

A toy git repo for the `code` task:

```sh
mkdir -p /tmp/overmind-smoke/repo && cd /tmp/overmind-smoke/repo
git init -q && printf 'def add(a, b):\n    return a - b\n' > calc.py
git add -A && git -c user.email=t@t -c user.name=t commit -qm "seed"
```

## What to run, and what to watch

Ordered so the cheapest failures surface first.

### 1 · First run — a knowledge task (~5–20¢)

Create a company, take the CEO it is founded with, give it a small research
task. This exercises M11 (knowledge execution), M14 (persona + domain in the
prompt), M18 (cost into the ledger) and M10 (the cage) all at once.

**Watch:**
- The session reaches `completed`, not `failed`. A sandbox denial shows up here
  first, as a session that dies early with an EPERM in its output.
- The task detail drawer shows **artifacts** — the agent produced files.
- The output is *in role*: an agent hired as `reviewer × media-av` should not
  read like a generic assistant. This is the only check that a person has to
  make; everything else a machine can.
- Budget: spend appears against the agent, and the reservation is **released**
  (`reserved_cents` back to 0).
- The recorded model is the one the agent was characterized for — **not
  `unknown`**. That was the defect the first pass found; this confirms the fix
  against a live run.

### 2 · A code task against the toy repo (~50¢–$2)

Connect `/tmp/overmind-smoke/repo` as the project workspace, hire a `builder ×
backend`, and ask for something small and checkable: *"fix the bug in `add`"*.

**Watch:**
- A **worktree** is created and the run happens inside it.
- **git works in the cage** — this is the M10 slice 2 fix meeting reality. The
  agent will likely run `git status` or `git diff`; a broken git shows as
  confused output or an early exit.
- The **diff** comes back in the drawer and contains the fix.
- The agent could not push: nothing reached the toy repo's `origin` (it has
  none, which is also fine — the point is no credential prompt and no hang).
- `deliverables/` — if the agent wrote a report as well as code, it is collected
  as an artifact **and** kept out of the diff (M17).

### 3 · Files in and out (~10–30¢)

Attach a small CSV to a knowledge task and ask for a summary plus a chart.

**Watch:**
- The file reaches the agent — it quotes real values from it, not invented ones.
- Outputs come back typed: a `.md` renders inline, a `.png` renders as an image,
  everything offers a download (M17).
- A subdirectory the agent creates keeps its path in the artifact list.

### 4 · A chat turn (~5–15¢)

Talk to the CEO: *"what should we do first?"*

**Watch:**
- The reply lands in the thread; any tasks it opened appear on the board.
- Cost lands in the ledger **for a chat turn** — this is M18's core claim and it
  has never been seen with real numbers.
- If the CEO proposes a team (M15), the proposal is drawn as an org chart and
  hires nobody until accepted.

### 5 · A meeting (~50¢–$2, 6+ turns)

Ask an agent something that needs a colleague, so it requests a room; approve it.

**Watch:**
- Nothing runs before approval.
- The transcript arrives turn by turn and reaches a **decision**.
- Cost is recorded **per turn**, and the per-turn budget gate holds.
- Optional and worth doing deliberately: set that agent's cap low first, so the
  room **pauses** mid-deliberation, then raise it and resume. That is ADR-0022's
  headline behaviour and it has only ever been seen against stubs.

### 6 · The audit chain

`GET /api/audit/verify` at the end — it should report valid over everything the
run produced.

## Stop conditions

Stop and look rather than pushing on if:

- A session fails with a permission error → the sandbox profile is missing
  something the real CLI needs. `OVERMIND_SANDBOX_ALLOW` widens it; note what
  was missing, because the default should probably learn it.
- Cost stays at zero after a completed run → the ledger is not seeing the
  envelope.
- The CLI hangs → likely a credential prompt that `GIT_TERMINAL_PROMPT=0` did
  not cover.
- A run fails with something the drawer cannot explain → read the envelope's
  `result`, which is where the adapter puts its own reason. That is now lifted
  into `last_error`, but only for shapes we have seen.
