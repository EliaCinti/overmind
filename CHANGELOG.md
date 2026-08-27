# Changelog

All notable changes to Overmind are recorded here, newest first. Overmind is developed milestone by milestone; each entry names the milestones it ships and the decisions behind them (`docs/adr/`). The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/) from `0.1.0` on — before `1.0`, minor versions may change behaviour.

## [Unreleased]

## [0.2.2] — 2026-08-28

The subscription sign-in, watched all the way through on a friend's machine. The flow reached "token created successfully" — and then declared failure.

### Fixed
- **A token the CLI printed is a token the flow keeps — whatever it looks like.** The scraper was anchored to the literal `sk-ant-oat`; on a real sign-in a TUI redraw landed *inside* that prefix and the subtype differed (`sk-ant-at01-…`), so the CLI's "✓ Long-lived authentication token created successfully" ended in "no token appeared in its output". The transcript is now walked as escape-free runs — a redraw anywhere, any `sk-ant-*` subtype, a label glued on by a cursor move: the credential is still found.
- **No credential ever leaves the module.** The failure message above carried the unrecognized token itself into `docker compose logs` — exactly where someone debugging would paste from. Everything the flow logs or hands to the interface (failure tails, the live tail, rejection notes) now passes a scrubber that blots out anything shaped like an `sk-ant-…` credential.

### Added
- **A skewed clock is named, not suffered.** OAuth codes are minutes-lived; a machine whose clock is minutes off the world refuses every code before it is pasted (a Docker Desktop VM woken from host sleep does exactly this). At sign-in the server measures its clock against the `Date` header of the API the CLI already talks to — UTC against UTC, so a timezone cannot cry wolf — and past two minutes of skew the card and the log say so, with the remedy per platform.
- **`docker-compose.yml` explains container time.** A commented `TZ=Europe/Rome` shows how to make logs read in local time — and says out loud that a timezone changes how time is *shown*, never what time it *is*.

## [0.2.1] — 2026-08-27

Fixes from the first fresh-machine install by someone who is not the author — the two-machine walk over Tailscale doing exactly its job.

### Fixed
- **The subscription sign-in survives the CLI's own retry.** On an OAuth 400 the CLI says "OAuth error … Press Enter to retry" — and the retry mints a *fresh* authorization URL (new PKCE challenge), so the old link is dead. The flow used to re-offer that dead link and every code from it answered 400, forever. Now the server recognizes the shape (even with the TUI's cursor-positioned words squashed together), presses Enter for the person, scrapes the URL printed *after* the restart and offers it with an honest note. The other shape — "Invalid code", same URL — still re-offers the paste box, and a rejection is judged on everything the CLI said since the code went in, not on a single read.
- **The sign-in narrates itself in `docker compose logs`.** Spawn, URL ready, code forwarded, refusal, retry, token stored — or exactly why not, in the CLI's own last words. When the flow stuck before, the logs had nothing to tell.
- **The brain never sinks the ship at build time.** A first-time installer's `compose build` died on the Wadachi layer (transient network). The image now degrades in loud steps — semantic → keyword-only → no memory — and the embedding-model bake is best-effort: a slower first run, never a broken build. Company deletion also survives the week's new schema (summaries, `depends_on`, `conversation_id`).

### Changed
- **The version answers without a login.** The first log line reads `overmind-server 0.2.1 listening…` (so `docker compose logs` says what is running), and `docker compose exec overmind overmind-server --version` says it on demand. `/api/health` still carries it for a signed-in caller — and stays redacted for anyone else.
- **`docker-compose.yml` says out loud that `down -v` deletes everything** — database, brains, audit chain, the agent's sign-in — and carries the one-line copy that backs the volumes up until export/restore ships as a real verb.

## [0.2.0] — 2026-08-27

### Added
- **The CEO runs the floor (M30, ADR-0042).** The chat plan gains `start` (start or relaunch existing tasks by title, through the same autonomy gates) and `after` (a task waits for another, inherits its deliverables as inputs, and starts when it completes). Digests may propose starts — each lands as an approval in the inbox, never as autonomous spend.
- **Tools in the agent's hand (M28, ADR-0036).** `OVERMIND_AGENT_TOOLS` declares MCP servers in the CLI's own shape; a tool is granted per agent as a structured, enforced trait, written into the run's and the turn's MCP config, offered in the hire dialog, listed at `GET /api/tools`. A tool can also be granted to — or taken from — an agent already hired (`POST /api/agents/{id}/tools`, the org chart's edit row), so the CEO can hire the team and the owner hands the one tool afterwards. First use: one modeler driving Blender through BlenderMCP (`docs/examples/agent-tools.blender.json`).
- **An `"exclusive"` tool fits one hand at a time.** The registry may declare `"exclusive": ["blender"]`; granting such a tool to a second active agent — at hire or after — is refused with the holder's name. The hire dialog marks it, and its empty state now points at how tools are declared.
- **Who pays is asked (M29, ADR-0037).** When a key is overriding a claude.ai login, every page offers *Let the plan pay*; the server keeps `ANTHROPIC_API_KEY` out of the agents' environment and asks the CLI again — refused with the reason when the key would still pay. The choice survives restarts and is undone from the org chart; `/api/health` carries `pay_with`.

### Fixed
- **From the CEO's plan to a running task (ADR-0038).** The CEO is told what each teammate holds (so Blender in one agent's hand is planned with, not around); a `code` task planned for a company without a repository is opened as `knowledge` (audited); and a planned task is offered by its agent's autonomy — started within budget, filed as a start approval in the inbox with approval, left for a human when propose-only. Before, a planned task for an *acts with approval* agent asked nobody and sat in `todo`.
- **An error Overmind can repair arrives with the repair.** A refused start now carries a machine-readable `remedy` (first: `grant_multimodal`) and the task detail offers it as one button — approve, Overmind acts, the start retries. Characterization is editable after hire: `POST /agents/{id}/traits` takes the hire's own validated patch and records a revision.
- **A task has a road back to the queue.** `in progress → todo` and `in review → todo` are valid moves: a run that must be redone returns to *todo*, where the start (and its approval gate) lives — before, redoing a reviewed task meant bouncing it through *blocked*.
- **The chat's files ride into the task.** A task the CEO opens from a conversation inherits that conversation's posted files: the run receives them and the task lists them. Before, "read the attached sketch" arrived with an empty run directory — the modeler built the whole plan from prose and said so.
- **The chat knows when an agent is answering, and runs one turn at a time.** `answering` rides on the conversation, so the typing dots survive a page switch; a message sent while the agent is answering waits and is read by the next turn instead of racing a second one. The inbox shows what waits on you alone, earlier items behind a toggle.
- **A plan the CEO wraps in a ```json fence, or pretty-prints over several lines, is found.** The parser read one line at a time; on the owner's first real brief the CEO answered with a sentence and a fenced, multi-line plan — shown raw in the chat, and the task it had planned was never opened. A balanced-brace scan (strings and escapes respected) now follows the line scan.

### Added
- **The CEO writes back on its own (ADR-0041).** When tasks born in a thread finish after your last word there, the thread's agent writes one unprompted update — or deliberately stays silent (`SKIP`), never twice for the same completions, never on top of a turn, debounced (`OVERMIND_DIGEST_DEBOUNCE_SECS`, default 180s; `OVERMIND_CEO_DIGEST=off` disables). An unprompted update opens no work.
- **The reply appears as it is written.** `--include-partial-messages` on the adapter; the readable reply streams into a live bubble with a cursor; spinner while working, dots only while writing, one narration line beneath.
- **The CEO reads the board before planning.** The chat turn's prompt carries the open tasks (status + assignee, newest first, capped at 40) with the instruction not to duplicate them — measured: every conversation round reopened the same lineages, three budget frames in a day.
- **A long conversation is compacted before it drowns the turn (ADR-0040).** Past `OVERMIND_CHAT_COMPACT_CHARS` (default 60k chars) the agent writes a handoff summary of the older messages; turns then read summary + recent tail, the summary is stored to the company brain too, and a quiet chip says it happened. Messages are never deleted; a failed compaction never eats the turn.

### Changed
- **`docs/TOOLS.md` — the complete tools manual.** Declaring, granting, the security model, a worked Blender example, other tools people attach, and a troubleshooting table; ready-made registries in `docs/examples/` (Blender, filesystem, browser). Linked from the README; the site wiki's Tools page mirrors it.
- **The work is visible while it happens (ADR-0039).** The adapter's own stream is read as it arrives: the chat's typing bubble names the tool in use or the agent's first words, and a running task's detail shows the same live line — instead of dots and silence for twenty minutes. The inbox closes itself after the last decision instead of surfacing the decided pile, and its list scrolls.
- **Agents' words render as the Markdown they are written in** — chat replies, meeting turns and decisions, what a task's session said: headings, lists, tables, code; raw HTML is never interpreted, links open in a new tab.
- **The chat composer grows with the draft** — one line at rest, about eight at most, then it scrolls; it returns to one line when the message is sent (`field-sizing: content` where the browser has it, measured elsewhere).
- **The first step is a choice:** a company holding only its CEO opens on the two-roads card (*tell the CEO the idea / build the team yourself*) and nothing else until a road is taken.

## [0.1.1] — 2026-08-22

### Fixed
- **The published image runs on Apple Silicon and arm Linux.** `0.1.0` shipped `linux/amd64` alone, and the very first `docker compose pull` on an Apple Silicon Mac answered *no matching manifest for linux/arm64*. The release workflow now builds both platforms natively — amd64 and arm64 on their own runners — and unites them under one manifest; `ghcr.io/eliacinti/overmind:0.1.1` (and `:0.1`, `:latest`) pull on either.

## [0.1.0] — 2026-08-22

The first release: a company of AI agents you can actually run, and let someone else into.

### The company
- A company with a **CEO you talk to**: it drafts the team (names, roles, reporting lines) and you approve the org in one click; it decomposes what you ask into tasks and puts agents on them. Specialists hand work to named teammates and escalate to the CEO (M11, M15, ADR-0016, ADR-0019).
- Agents are **archetype × domain** — a *reviewer* in *security*, a *writer* in *media* — two clicks, zero prose; traits compose general→specific and compile to server-enforced config (M14, ADR-0005, ADR-0021).
- An **org chart** rooted at you, with real reporting lines and no cycles; hire under any node, reassign, retitle (M5).
- **Tasks** for code (an isolated git worktree and branch per run, a diff to review) and for documents (no repository; artifacts to download); attachments in, artifacts out; meeting transcripts kept (M2, M12, M17, ADR-0017).
- **Meetings**: an agent requests a room, you allow it; bounded round-robin deliberation with a turn cap; the decision is audited, stored to memory and injected into every participant's next prompt; one pending request per agent, three per company (M13, M13.5, ADR-0020).
- **One inbox** for everything an agent wants you to know or decide, answered inline; live updates over a WebSocket (M12, M16).
- **Two languages**, English and Italiano, stored on the company so agents write in it from the first second (M16).

### Memory
- **Organizational memory over MCP**, provider-agnostic: `get_context` before work, `store_memory` after, `store_decision` for meetings; graceful degradation by contract — no provider, broken provider, tasks run identically (M7, ADR-0003).
- **One brain per company**, provisioned as a directory and a `BRAIN_DIR`; **on by default in the image** (Wadachi pinned, semantic model baked in); every brain is **born knowing who the company is** (M8, M21, ADR-0024, ADR-0031).
- A **Memory view** with provenance (which task or meeting produced each memory) and semantic search; agents reach memory read-only through Overmind's own MCP endpoint (M9, ADR-0025, ADR-0027).
- **Change awareness**: a watermark at checkout, and a notification when two runs wrote about the same thing without seeing each other (ADR-0026).

### Money
- **Per-agent monthly caps**, atomic with checkout, for task runs and conversational turns alike (M6, M18, ADR-0012, ADR-0022).
- **Two economies, detected not configured**: an API key (the cap is real money) or a Claude subscription (amounts are equivalents; both plan windows shown with their state and reset), with the adapter's `--max-budget-usd` as the brake and the honesty that it overshoots (M20, ADR-0030).
- **The estimate learns from the ledger**: the reservation is the agent's own last ten costs of the same kind, leaning dear; the flat default stands, visibly, below three samples (M26, ADR-0035).
- Sign into a subscription **from the product** (M23).

### Trust
- **A hash-chained, append-only audit** of every mutation, verifiable at `GET /api/audit/verify`; the acting user rides inside the hashed payload; approvals and meetings say *who* decided, read off the chain (M1, M24, M25).
- **The cage**: `sandbox-exec` deny-by-default on macOS; in the container an unprivileged uid plus Landlock where the kernel has it; credentials isolated from the agent; one predicate decides whether the CLI may skip its permission prompts (M10, M19, ADR-0023, ADR-0029).
- **The door**: an owner claimed at first run (argon2id, hashed sessions, rate-limited login, CSRF belt); **invites** (single-use, seven days, hashed) from the second person on; **membership** as the filter on every company surface, bare ids included; a members list; **delete a company** by typing its name — the audit chain stays (M24, M25, ADR-0032, ADR-0033, ADR-0034).
- Overmind as an **MCP server** for outside callers, with labelled, revocable tokens — filing is a request, starting stays yours (ADR-0028).

### Running it
- A **self-contained Docker image** (agent CLI and brain inside), named volumes for the data and the agent's sign-in, `EXTRA_APT_PACKAGES` for your toolchains, repositories under `/repos`; from source on macOS with `cargo run` (M19, M22, M23).
- **Several people, one company, over Tailscale**: bind to the tailnet address, claim early, invite, add to a company, decide with a name. The host can be a Mac (native or image), Linux or Windows (the image); the colleague needs a browser.
- Where each person left off is remembered by the server, per user; approval summaries speak the company's language (M23).

### Known limits, stated
The threat model (`docs/THREAT-MODEL.md`) says what is not defended against: anyone with the machine, a malicious adapter or memory binary you configured, the network. No quorum on meeting requests and declared-but-unpoliced permissions are deliberate; membership is organizational, not adversarial; the plan's remaining percentage is not visible headless; the adapter's brake overshoots (measured 2.6× at a five-cent ceiling).

[0.1.1]: https://github.com/EliaCinti/overmind/releases/tag/v0.1.1
[0.1.0]: https://github.com/EliaCinti/overmind/releases/tag/v0.1.0
