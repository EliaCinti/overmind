# What comes after 0.1

*Written 22 August 2026, the day 0.1.1 shipped. The [roadmap](ROADMAP.md) records what was built and how each milestone was accepted; this file says what comes next, in the order it should come, and why that order. When a milestone here opens, it moves to the roadmap as `in-progress` and this file shrinks.*

## Where 0.1 leaves us

Twenty-seven milestones: a company with a CEO you talk to, memory, a hash-chained audit with the actor inside, budgets in both economies with an estimate learned from the ledger, the cage, the door, invites and membership, a published multi-arch image. What is **not** yet true, and what this plan is built around:

1. **Nobody has used it for a week.** Every milestone was closed by tests and a live acceptance; three of them (M23 daily use, M24/M25 the two-machine walk, M27 the fresh-machine install) still carry *the owner's half* of their acceptance. Until those walks happen, the next milestone is a guess.
2. **A code run's diff has nowhere to land.** The task drawer shows the diff; the branch stays in the worktree; the product has no verb for "take it" — the person merges by hand with git. That is the loop's missing last step.
3. **The data has no way out.** One owner, one SQLite, one brain per company, one Docker volume — and today an unplugged SSD proved the volume can vanish under a running factory. There is no backup, no export, no restore.
4. **The walks are by hand.** Every UI acceptance so far was a person (or a Playwright script run once from a laptop). Nothing in CI opens a browser; a regression in the door, the onboarding or the inbox would ship green.
5. **The audit chain is trusted but unseen.** The actor flows to the API (`actor_name`), approvals and meetings say who decided — but there is no page that shows the chain itself, and no history of decided approvals once they leave the inbox.
6. **It is a desk product.** Decisions reach a person who is mostly away from the desk, and the interface assumes a mouse and a wide screen; there is no way to be told on a phone that a run waits, and no secure context on the tailnet for a phone to be told in.
7. **It reaches only your tailnet.** The door was designed for a network you own; a team, a phone on cellular, someone you will never add to a tailnet cannot get in without exposing a port the door was not hardened for — no second factor, no per-IP limit, a claim anyone can take on an unclaimed instance.

## The order, and why

> **23 Aug:** *Tools in the agent's hand* jumped the queue as **M28** — a real need named it (a Blender company for the owner's house, ADR-0036), which is exactly the rule at the bottom of this file. The milestones below keep their names; their numbers shift by one when they open.

### A week of use *(the owner's milestone)*
Not a development milestone: the owner uses Overmind for real work for a week, does the **two-machine walk over Tailscale** with a friend (closes M24 and M25), has the friend install it on a fresh machine with `docker compose pull && up` (closes M27), and writes down every friction **in the order it bit**. Those frictions are the backlog for what follows, ahead of anything below — the roadmap's own doctrine since M23: *dogfood first, then fix what actually hurt.*

**Accept:** M23, M24, M25, M27 marked `done` on the owner's word; a list of frictions, ranked.

### Backup and restore
Today's incident, turned into a verb. One API and one button: **export** a company — or the whole instance — as a single archive (the database, every brain, the subscription token, attachments and artifacts), and **restore** it into a fresh instance. The archive is what you keep on another disk; the restore is what makes the image disposable. Plus the documented, tested recovery of a Docker volume gone bad.

- Export is consistent: taken inside a read transaction (SQLite backup API), brains copied after the database, the audit chain verified before the archive is sealed and the verification written into it.
- Restore refuses an archive whose chain does not verify, says so, and never touches the running data until the archive is accepted.
- The owner's verb, like billing: an export is everyone's data.

**Accept:** export on one machine, restore on another, the door, the companies, the memories and the chain all intact; the threat model gains a row.

### From diff to landed
The loop's last step. After review, a person can **land** a code run: merge its branch into the workspace's default ref (fast-forward or merge commit, the repo's history kept honest), or — when the repository has a remote and `gh` is signed in — **open a pull request** with the task's brief as the description and the diff as the body. Both are the human's verb, audited with the actor; an agent never lands its own work. Conflicts are reported, not resolved by a machine.

**Accept:** a code task completes, the diff is reviewed, *Land* merges it into the default branch and the board shows it; with a remote, *Open a PR* produces a PR whose description names the task. Held by tests against a real local repository.

### The browser walk, in CI
Every acceptance walk that has ever been done by hand, done by a machine on every pull request: the door (claim, login, invite, sign-up), onboarding (found, language, skip repo), the company (delete by name), members, a gated task through the inbox with the actor's name, the org view's life-line. Playwright against the real server with a stub adapter — no API key, no money — on the Linux runner; screenshots on failure. The wiki's guides become executable.

**Accept:** the walk is a CI job, green on a PR that changes nothing and red on one that breaks the door.

### The chain, seen
A page for the audit chain: the events of a company, each with its actor's name, kind, time and payload; the verify result at the top; filters by kind and actor. And a **history** of approvals and meetings — decided, by whom, when — so the inbox can stay a place for what is pending. The thing the product is proudest of, finally visible without `curl`.

**Accept:** the page shows the chain and says "verified"; break the chain in the test and the page names the block.

### Overmind in your pocket
The owner's ask, and the right shape for it. A person who runs a company of agents is mostly *away from it* — and what reaches them is a decision: approve a start, allow a meeting, answer the CEO, glance at the board. That is a phone's job. Overmind is already reachable from a phone over Tailscale (iOS and Android have it), with no cloud in between; the hard part of "an app" is done. The rest comes in stages, cheapest and most honest first:

- **A · HTTPS on the tailnet, verified.** A service worker and push need a secure context, and the documented way to reach Overmind today is plain HTTP on the tailnet address. `tailscale serve` gives the machine an HTTPS name and a certificate for free; verify it against the door (cookie `Secure`, `OVERMIND_COOKIE_SECURE=on`), the WebSocket's origin guard and the live updates — and write the result into the wiki, where today it is only promised for a reverse proxy.
- **B · The decision surfaces, mobile-first.** The inbox, the meeting room, the chat and the board laid out for a hand: one column, thumb-reachable actions, nothing that needs a hover. Installable as a PWA — an icon on the home screen, full screen, no store, no signing, no review.
- **C · "Waiting on you", pushed.** Web Push (VAPID keys minted by the server, kept under the data dir) for exactly the notifications that wait on a person; tapping one opens the item. Nothing else pushes — the phone is for deciding, not for watching.
- **D · A native shell, only if B and C prove insufficient** — and, if it comes, **built with Overmind**: a company *Overmind Mobile*, a workspace on the app's repository, the CEO drafting the team, every diff reviewed by the owner and landed through M30's verb, the costs in the ledger. Documented as a walk, not claimed as a demo: the strongest proof the product can give that it does real work is to ship a piece of itself.

**Accept:** from a phone on the tailnet, over HTTPS: sign in, get a push that a run waits, approve it, see the board move — without an app store. D has its own acceptance when it opens.

### The door on the open internet
M33 reaches you and the two friends on your tailnet. Reaching *more* people — a team, a phone on a cellular network, someone you will never add to a tailnet — means a port on the internet, and there the security stops being "the network is mine" and becomes "the door holds on its own". The principle that does not move: **never a cloud of ours in the middle.** The server stays yours; the phone and the others come in through the same door. What changes is how much that door has to hold, and what it is honest to say about it.

- **A · The ways to widen reach, cheapest and safest first, each verified and written down.** *Share the tailnet*: invite a device into it (Tailscale node sharing; identity stays at the network layer, nothing public). *Tailscale Funnel*: a public HTTPS name through Tailscale's relay, TLS terminated for you, the door carrying the whole weight — no certificate to own, a bandwidth ceiling to know about. *A VPS with Caddy*: a domain you own, TLS by Caddy, `OVERMIND_COOKIE_SECURE=on`, the image pulled from GHCR — a compose file with Caddy in it, ready to copy. Three doors, one server, your data still on it.
- **B · A door that holds on a public port.** Today's door was designed for a tailnet: argon2id, hashed sessions, invites, a per-name rate limit. A public port needs more, and each is a test before it is a feature: a **second factor** for every account — passkeys (WebAuthn) first, TOTP as the fallback — because a password alone on the internet is a promise nobody should make; **per-IP rate limiting** that reads the real client behind a proxy; a **session list with revoke** ("this phone, that laptop"); security headers (HSTS, CSP) set by the server rather than hoped for from a proxy; and a **claim that cannot be stolen**: an unclaimed instance is open by construction, so an instance that is not on loopback prints a one-time setup code at first boot and refuses any claim without it.
- **C · The threat model, rewritten as the boundary moves.** "The door guards the port; nothing guards a hostile host" stays true; what changes is who can reach the port. Every new claim in B gets its adversarial test in `the_door.rs` — a stolen password without the second factor, a forged claim on a public port, a revoked device that keeps trying — and the model says which exposure (tailnet, Funnel, VPS) each protection is *for*.
- **D · Distribution, without a store of our own.** The image is public already; this adds the one-file VPS template, the wiki page that walks the three doors, and — for the phone — M33's PWA over the same public door, so "install it" is a link, not an account with us.

**Accept:** the same Overmind reached three ways — tailnet, Funnel, VPS — by a person with a passkey on a phone off the tailnet; an attacker with the password alone stays out; an unclaimed instance on a public port cannot be claimed without the code. Threat model rows for each, named by their tests.

### Then, when someone needs them
- **Per-company roles and member removal** — deferred in ADR-0033 until a real need names them; the two-machine walk will say whether it has.
- **Quorum on meeting requests** — deliberately absent (seconding would burn a turn per invitee before the human answers); revisit only if the per-agent and per-company limits prove insufficient in use.
- **Declared permissions, policed** — `repo:write`, `web:read` and the rest are compiled into the brief, not enforced; enforcing them means the cage speaking the permission vocabulary. Large; worth it only once agents are trusted with more than they are today.
- **Native Linux and Windows** — the image is the path and CI tests it; native builds would prove a port we are not doing.
- **A plugin system, role templates from a marketplace** — icebox; not before a second person has asked.

## Small debts from the first live walk (23 Aug 2026)

Found while the owner drove the Casa San Vito company; each is small, none is urgent, all are real:

- **Characterization is barely editable after hire.** Tools got their endpoint and chips (ADR-0036); `multimodal` did not — a modeler hired without it cannot be handed a sketch until someone edits the database. The right shape is one `POST /agents/{id}/traits` taking a validated `TraitsPatch` (revision `patch`, same rules as hire), and the org chart's edit row growing the few fields that matter: multimodal, autonomy, model.
- **Duplicate attachments read twice.** The same file uploaded twice to a thread is listed twice ("BozzaCasa.jpeg, BozzaCasa.jpeg") and copied over itself in the run dir. Dedup by (filename, size) at listing time, or say "×2".
- **A manual board move to *in progress* runs nobody.** The transition is legal but inert — either offer the start when the column changes, or say plainly that starting lives on the task.

## How this file is used
Pick the top milestone, write its ADR if it changes a boundary or a contract, open it in the roadmap as `in-progress`, close it the way every milestone was closed — tests, a live walk, the owner's word — and come back here to take the next one. Frictions from use jump the queue.
