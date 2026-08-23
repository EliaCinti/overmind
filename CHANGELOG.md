# Changelog

All notable changes to Overmind are recorded here, newest first. Overmind is developed milestone by milestone; each entry names the milestones it ships and the decisions behind them (`docs/adr/`). The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/) from `0.1.0` on — before `1.0`, minor versions may change behaviour.

## [Unreleased]

### Added
- **Tools in the agent's hand (M28, ADR-0036).** `OVERMIND_AGENT_TOOLS` declares MCP servers in the CLI's own shape; a tool is granted per agent as a structured, enforced trait, written into the run's and the turn's MCP config, offered in the hire dialog, listed at `GET /api/tools`. A tool can also be granted to — or taken from — an agent already hired (`POST /api/agents/{id}/tools`, the org chart's edit row), so the CEO can hire the team and the owner hands the one tool afterwards. First use: one modeler driving Blender through BlenderMCP (`docs/examples/agent-tools.blender.json`).
- **Who pays is asked (M29, ADR-0037).** When a key is overriding a claude.ai login, every page offers *Let the plan pay*; the server keeps `ANTHROPIC_API_KEY` out of the agents' environment and asks the CLI again — refused with the reason when the key would still pay. The choice survives restarts and is undone from the org chart; `/api/health` carries `pay_with`.

### Changed
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
