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

> **3 Sep 2026 — where the work stands, in the order it should be finished.**
> Written after the owner said development had slowed and asked for a plan to
> follow. Three horizons, and the rule that governs them: *finish what is half
> done before opening anything new.*
>
> **Now.** PR #91 (M33, and M32's script) → tag **0.2.4**, which is the release
> that publishes the new compose file *and* the stamped installers → the nginx
> redirect for `/install.sh` on the VPS, **ahead of the SPA's catch-all**, or
> `curl` pipes the landing page into a shell → then, and only then, move the
> docs to the release asset URL, which until that tag still serves 0.2.3's
> compose and its named volumes.
>
> **Then**, the debt already measured and written down: *Updating, and the way
> back* (below); the `ClaimedOwner` extractor (below, now defence in depth
> rather than a hole, since M32 slice A closed the wall); and M32's slice C,
> the free graphical road, to be **decided on evidence about who actually
> installs it** rather than on taste.
>
> **After that**, the queue below in its existing order: from diff to landed,
> the browser walk in CI, the chain seen, Overmind in your pocket, the door on
> the open internet.
>
> **And the owner's own:** raise the cap that is holding *TravelAgency* — one
> agent at €50.38 against €50.00, six refused starts in two hours — and say the
> word on M25, open since 29 August.


> **23 Aug:** *Tools in the agent's hand* jumped the queue as **M28** — a real need named it (a Blender company for the owner's house, ADR-0036), which is exactly the rule at the bottom of this file. The milestones below keep their names; their numbers shift by one when they open.

### A week of use *(the owner's milestone)*
Not a development milestone: the owner uses Overmind for real work for a week, does the **two-machine walk over Tailscale** with a friend (closes M24 and M25), has the friend install it on a fresh machine with `docker compose pull && up` (closes M27), and writes down every friction **in the order it bit**. Those frictions are the backlog for what follows, ahead of anything below — the roadmap's own doctrine since M23: *dogfood first, then fix what actually hurt.*

**Accept:** M23, M24, M25, M27 marked `done` on the owner's word; a list of frictions, ranked. *29 Aug 2026: M23, M24, M27 (and M29) closed on his word; the frictions are the list above. M25 waits for his word on the shared-company half of the walk.*

### Backup and restore
*Opened 29 Aug 2026 as **M31** ([ADR-0044](adr/0044-the-archive-is-the-instance.md)); the milestone, its slices and its Accept live in the roadmap now. What was sketched here — the archive, the verified chain, the restore into a fresh instance — is there in its decided form; the token is sealed by a passphrase rather than carried in the clear, and the scratch dirs stay out.*

### Updating, and the way back
*Measured 1 Sep 2026, while accepting M31.* Updating **forward** is already safe and needs nothing: `docker compose up -d --pull always` replaces the container, never the volumes; the compose declares `name: overmind`, so an update run from any directory finds the same data; and migrations are cumulative, so an instance on 0.1.0 can go straight to the newest with no stepping stones. Updating is opt-in — nothing moves until that line is run — and staying put is a deliberate edit of `image:` ([ADR-0043](adr/0043-the-compose-file-is-the-installers.md)).

**Going back is where it stops.** `sqlx::migrate!` runs at boot and there is no schema-version guard, so an older binary meeting a database a newer one has migrated exits with `Migrate(VersionMissing(...))` and refuses to start. Failing closed is right — nothing is corrupted — but three things follow that we tell nobody: there is **no downgrade**; **an archive is not a rollback**, because a restore needs an *empty* instance, so going back means emptying the volume and restoring into it; and **nothing prompts an archive before an update**, so the person who needs one is exactly the person who does not have one. The error itself names a migration number and no remedy.

M31 built the machinery; what is missing is the moment it is used on its own. The shape, cheapest first: at boot, when migrations are about to be applied that were not there before, **take an archive into the configured backup folder** (`OVERMIND_BACKUP_DIR`, whose default is `<data>/backups/` — the boot-time archive must follow the configuration, not the default), **named for the version being left** — the one instant when the need is certain and the data is still the old version's. And **make the refusal speak**: record which version last opened the database, so an old binary can say *"this data was written by a newer Overmind (0.3.x); this is 0.2.3 — update again, or start empty and restore an archive"* instead of a migration number.

**Accept:** update across a migration and find an archive of the previous version waiting, without having asked for it; roll the image back and read a sentence that says what happened and what to do; empty the volume, restore that archive, and be back where you were.

### Who pays, and on which model *(from the org view)*
*Found in use on 3 Sep 2026, on a Mac with `ANTHROPIC_API_KEY` in the shell.* Overmind went straight to the key and never offered the subscription. The striking part is not that the road is missing — **it is built, and locked.**

**Both halves of the subscription road already work, and the ordinary setup can reach neither.** `SignInNotice` holds a full guided sign-in — `claudeAuthStart`, the clock-skew warning, the code paste — and shows itself only when there is *no* way to pay at all (`unknown_kind == "not_signed_in"`). `PayerNotice` holds the switch, and shows itself only when a key is *beating* a login (`kind == "key" && overrides_login`). A key present and no login satisfies neither condition, so the person sees nothing, and the product's own answer — connect the plan, then choose it — is unreachable from the product. Today it costs `claude setup-token` in a shell.

Underneath, the machinery is sound and needs no change: `prefer_plan` writes `<data>/pay-with-plan`, and while that marker exists `sandbox.rs` strips `ANTHROPIC_API_KEY` from every command run as the agent, so the CLI bills the plan. **That is also why the order matters** — the switch only hides the key; hiding it with no login behind it leaves the agent with no credentials at all. Sign in first, choose second. The current gate enforces that ordering by accident, at the price of hiding the road.

**The org view is where this belongs**, not in notices that appear when the system decides something is wrong. `EconomyNote` already names who pays, above the chart, and already carries the *undo* (`back to key`, whenever the plan was chosen). It is half a control today: the way back exists, the way forward does not. Make it symmetric — paying with a key offers *connect the subscription* (the guided flow, moved here) and then *let the plan pay*; paying with the plan keeps its undo. One place, both directions, reachable whether or not anything is contested.

**And the model catalogue does not know who pays.** `Model` is `{ id, display_name, vision }` — nothing about entitlement. The hire dialog offers every model whatever the economy is, so an agent can be put on a model the subscription does not grant and the refusal arrives at run time, in the adapter's words, as a failed task. The choice also exists **per agent** only (a trait, passed as `--model`): a company cannot say "everyone runs on Sonnet unless I say otherwise", so ten agents are ten decisions and a change of mind is ten edits.

In order of what it costs a person today:
1. **The payer, from the org view** — the symmetric control above. Small, and it unlocks work already written.
2. **Entitlement in the catalogue.** Each model declares which economies reach it; the hire dialog offers what *this* instance can run and greys the rest with the reason, rather than letting a task fail to teach it. The plan's limits are not readable from outside the CLI ([ADR-0030](adr/0030-who-pays.md)), so this is a declared table kept honest by the probe that already asks who pays.
3. **A company default**, with an agent overriding it only for a reason, and the org view showing who differs.
4. **A warning before the switch**, once entitlement exists: moving to a plan that does not grant a model somebody is already on should be said before the change, not at the next run.

**Accept:** on an instance paying with a key, the org view offers the subscription, the guided sign-in completes there, and choosing the plan changes who pays without touching a shell; the hire dialog does not offer a model this instance cannot run; a company sets a default and a new agent inherits it.

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
- **An unclaimed instance is open to more than the claim.** `require_owner` returns `Ok` while `users` is empty and the wall waves the whole API through — right for claiming, wrong for everything else: `/economy/pay-with`, `/claude-auth/start`, `/claude-auth/code` and `/auth/invites` all answer a stranger who reaches the port of a box nobody has claimed yet. M31's export took its own `require_claimed_owner` predicate rather than widen the hole; the general fix is a `ClaimedOwner` extractor (or a `require_owner` that tells "claimable" from "owner-only"), and it wants an ADR first — which routes stay open before the claim *is* the first-run flow.
- **The CEO reports a deliverable it cannot see.** Twice on 27 Aug the CEO's unprompted update opened with "its `ARTIFACT.md` is not in the shared folder — I report what it declares, not what it contains", while the file sat at `data/artifacts/<session>/ARTIFACT.md` (23 KB and 21 KB) and in `task_artifacts`. The CEO was honest about the gap; the gap should not exist. Either the path it is told to read is not where artifacts land, or it is told nothing and goes looking. The digest that wakes it should carry the artifact's path — and the text, when it is short — in the same words the runner uses.

## Frictions from the two-machine walk (27 Aug 2026, in the order they bit)

The owner and a friend ran the walk: a fresh install on the friend's machine, shared over the tailnet. What bit, ranked:

1. **The subscription sign-in stuck.** The friend's code exchange answered an OAuth 400; the CLI said "Press Enter to retry" — a shape the flow did not know. The retry mints a *fresh* URL (new PKCE challenge), so re-offering the old link loops on 400 forever; the flow ended with "the CLI finished but no token appeared". *Fixed on `build-never-sinks`: the OAuth-error shape is recognized, Enter is pressed for the person, the fresh URL is scraped and offered with an honest note.*
2. **Everything rides one Docker volume.** `docker compose down -v` — or a lost disk — is total loss. The *Backup and restore* milestone below is the verb; until it lands, the wiki should say plainly which volumes hold the data and that `-v` destroys them.
3. **Plain HTTP on the tailnet.** The documented sharing path is `100.x.y.z:7070` in the clear. `tailscale serve` is the free fix (stage A of *Overmind in your pocket*); its verification should jump ahead of the rest of that milestone.
4. **The logs say almost nothing.** Two startup lines, then errors only — no timestamps, no run lifecycle. When the sign-in stuck, `docker compose logs` had nothing to tell. The sign-in flow now narrates itself; the rest of the server deserves the same: timestamped lifecycle lines (task started, agent spawned, turn finished, cost recorded) — one focused PR.
5. **Updating is ambiguous while the compose declares both `image:` and `build:`.** A friend's `docker compose pull` did not visibly land the new version (a local `up --build` had claimed the `latest` tag; Compose's behavior around buildable services varies) and the fix was "delete everything and redownload" — a sledgehammer nobody should need. Split the worlds: the default `docker-compose.yml` carries only the published image (the installer's experience), a `docker-compose.build.yml` override carries `build: .` for developers; document `docker compose up -d --pull always` as the one update command. *Fixed on `compose-is-the-installers`: the default file names only the image, `docker-compose.build.yml` carries `build:` (tagged `overmind:local`), and the compose file rides every release as an asset.*
6. **Install is still "clone the repo or copy the compose" — and it assumes `docker` exists.** The Quickstart's first word is `git clone`; nothing says what provides Docker per OS (asked by the owner himself, 28 Aug). One file and one command is the honest target: publish `docker-compose.yml` as a release asset, a three-line wiki page per OS that OPENS with the prerequisite — Linux: the docker engine package; macOS: Docker Desktop or, terminal-only, Colima (`brew install colima docker docker-compose`, `colima start`); Windows: Docker Desktop over WSL2 — then download, `docker compose up -d --pull always`, open `localhost:7070`; the per-OS knobs (UID/GID mounts on Linux, TZ display, the tailnet port) named on that page. What no package can absorb — Docker itself, the host clock, an active subscription — the container keeps learning to *detect and say* (0.2.1 says its version; 0.2.2 names a skewed clock). *The asset and the `curl` half landed with 5 (ADR-0043); the per-OS pages remain — and once a release carries the asset, the README's `curl` should move from raw `main` to `releases/latest/download/docker-compose.yml`, so the file people get is the one tested with the image they get.*

## Local models: Overmind off the meter (researched 27 Aug 2026)

The owner's ask: run Overmind's agents on local LLMs — for privacy and to
escape token windows. Research says the road is short, because the design
already leaves it open:

- **The cheap path needs no new code.** Since early 2026 llama.cpp, Ollama
  and LM Studio all speak the **Anthropic Messages API**; the Claude Code CLI
  follows `ANTHROPIC_BASE_URL`. Point the CLI at a local server (or a LiteLLM
  router) and today's Overmind runs unchanged — tools, MCP memory, personas
  and all — with `OVERMIND_ECONOMY` declared. This is the first walk to do.
- **First-class support is one milestone ("local is an economy"):**
  1. `OVERMIND_AGENT_ENV` (or per-model `base_url` in the catalog) so the
     server injects the routing per run instead of the operator exporting it;
  2. local models in the **model catalog**, so the `model` trait per agent
     buys a **mixed fleet** — CEO on Claude, workers on Qwen — via a LiteLLM
     route table (claude-* → Anthropic, qwen-* → llama.cpp);
  3. a third economy kind `local`: no money, no plan windows — the honest
     wording for caps that brake loops but bill nobody;
  4. docs: hardware tiers (below), model picks, the router recipe.
- **Hardware tiers** (researched Aug 2026, M5-generation Macs just announced):
  *Basic* ~€2–3k (Mac Mini/Studio M4-class 64–128GB, or 1× RTX 5090 32GB):
  Qwen3-30B-A3B / gpt-oss-20b class — one agent at a time, good drafts.
  *Normal* ~€5–7k (Mac Studio M5 Max 256GB, or 2× 5090): gpt-oss-120b /
  Qwen3-235B-A22B quantized — a small org's daily driver.
  *Max* ~€15k+ (Mac Studio M5 Ultra 512GB, ~Oct 2026): DeepSeek-class 671B
  at 4-bit, Kimi K2-class 1T at ~2-bit, 5–15 tok/s. Caveat that matters to
  Overmind specifically: agent prompts are long, and unified-memory Macs pay
  it in **prefill**; MoE models and prompt caching are the mitigations.

## How this file is used
Pick the top milestone, write its ADR if it changes a boundary or a contract, open it in the roadmap as `in-progress`, close it the way every milestone was closed — tests, a live walk, the owner's word — and come back here to take the next one. Frictions from use jump the queue.
