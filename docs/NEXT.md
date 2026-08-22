# What comes after 0.1

*Written 22 August 2026, the day 0.1.1 shipped. The [roadmap](ROADMAP.md) records what was built and how each milestone was accepted; this file says what comes next, in the order it should come, and why that order. When a milestone here opens, it moves to the roadmap as `in-progress` and this file shrinks.*

## Where 0.1 leaves us

Twenty-seven milestones: a company with a CEO you talk to, memory, a hash-chained audit with the actor inside, budgets in both economies with an estimate learned from the ledger, the cage, the door, invites and membership, a published multi-arch image. What is **not** yet true, and what this plan is built around:

1. **Nobody has used it for a week.** Every milestone was closed by tests and a live acceptance; three of them (M23 daily use, M24/M25 the two-machine walk, M27 the fresh-machine install) still carry *the owner's half* of their acceptance. Until those walks happen, the next milestone is a guess.
2. **A code run's diff has nowhere to land.** The task drawer shows the diff; the branch stays in the worktree; the product has no verb for "take it" — the person merges by hand with git. That is the loop's missing last step.
3. **The data has no way out.** One owner, one SQLite, one brain per company, one Docker volume — and today an unplugged SSD proved the volume can vanish under a running factory. There is no backup, no export, no restore.
4. **The walks are by hand.** Every UI acceptance so far was a person (or a Playwright script run once from a laptop). Nothing in CI opens a browser; a regression in the door, the onboarding or the inbox would ship green.
5. **The audit chain is trusted but unseen.** The actor flows to the API (`actor_name`), approvals and meetings say who decided — but there is no page that shows the chain itself, and no history of decided approvals once they leave the inbox.

## The order, and why

### M28 — A week of use *(the owner's milestone)*
Not a development milestone: the owner uses Overmind for real work for a week, does the **two-machine walk over Tailscale** with a friend (closes M24 and M25), has the friend install it on a fresh machine with `docker compose pull && up` (closes M27), and writes down every friction **in the order it bit**. Those frictions are the backlog for M29+, ahead of anything below — the roadmap's own doctrine since M23: *dogfood first, then fix what actually hurt.*

**Accept:** M23, M24, M25, M27 marked `done` on the owner's word; a list of frictions, ranked.

### M29 — Backup and restore
Today's incident, turned into a verb. One API and one button: **export** a company — or the whole instance — as a single archive (the database, every brain, the subscription token, attachments and artifacts), and **restore** it into a fresh instance. The archive is what you keep on another disk; the restore is what makes the image disposable. Plus the documented, tested recovery of a Docker volume gone bad.

- Export is consistent: taken inside a read transaction (SQLite backup API), brains copied after the database, the audit chain verified before the archive is sealed and the verification written into it.
- Restore refuses an archive whose chain does not verify, says so, and never touches the running data until the archive is accepted.
- The owner's verb, like billing: an export is everyone's data.

**Accept:** export on one machine, restore on another, the door, the companies, the memories and the chain all intact; the threat model gains a row.

### M30 — From diff to landed
The loop's last step. After review, a person can **land** a code run: merge its branch into the workspace's default ref (fast-forward or merge commit, the repo's history kept honest), or — when the repository has a remote and `gh` is signed in — **open a pull request** with the task's brief as the description and the diff as the body. Both are the human's verb, audited with the actor; an agent never lands its own work. Conflicts are reported, not resolved by a machine.

**Accept:** a code task completes, the diff is reviewed, *Land* merges it into the default branch and the board shows it; with a remote, *Open a PR* produces a PR whose description names the task. Held by tests against a real local repository.

### M31 — The browser walk, in CI
Every acceptance walk that has ever been done by hand, done by a machine on every pull request: the door (claim, login, invite, sign-up), onboarding (found, language, skip repo), the company (delete by name), members, a gated task through the inbox with the actor's name, the org view's life-line. Playwright against the real server with a stub adapter — no API key, no money — on the Linux runner; screenshots on failure. The wiki's guides become executable.

**Accept:** the walk is a CI job, green on a PR that changes nothing and red on one that breaks the door.

### M32 — The chain, seen
A page for the audit chain: the events of a company, each with its actor's name, kind, time and payload; the verify result at the top; filters by kind and actor. And a **history** of approvals and meetings — decided, by whom, when — so the inbox can stay a place for what is pending. The thing the product is proudest of, finally visible without `curl`.

**Accept:** the page shows the chain and says "verified"; break the chain in the test and the page names the block.

### Then, when someone needs them
- **Per-company roles and member removal** — deferred in ADR-0033 until a real need names them; the two-machine walk will say whether it has.
- **Quorum on meeting requests** — deliberately absent (seconding would burn a turn per invitee before the human answers); revisit only if the per-agent and per-company limits prove insufficient in use.
- **Declared permissions, policed** — `repo:write`, `web:read` and the rest are compiled into the brief, not enforced; enforcing them means the cage speaking the permission vocabulary. Large; worth it only once agents are trusted with more than they are today.
- **Native Linux and Windows** — the image is the path and CI tests it; native builds would prove a port we are not doing.
- **A plugin system, role templates from a marketplace** — icebox; not before a second person has asked.

## How this file is used
Pick the top milestone, write its ADR if it changes a boundary or a contract, open it in the roadmap as `in-progress`, close it the way every milestone was closed — tests, a live walk, the owner's word — and come back here to take the next one. Frictions from use jump the queue.
