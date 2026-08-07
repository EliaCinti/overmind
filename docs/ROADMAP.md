# Roadmap

> Single source of truth for "what's next". The MVP vision is big; the working unit is the **milestone**, and a milestone is done only when its acceptance criteria pass. We do not start milestone N+1 with N half-finished.
>
> Status legend: `todo` · `in-progress` · `done`

## M0 — Foundations `done`
Repo, docs, decisions.
- [x] Repo + git init
- [x] VISION / ARCHITECTURE / ROADMAP / ADRs 0001–0004
- [x] Rust workspace skeleton (`overmind-server`), CI workflow (fmt, clippy, test), `cargo run` serves `/health` — verified locally (fmt+clippy+test green, endpoint answers)
- [x] First commit; public GitHub repo ([EliaCinti/overmind](https://github.com/EliaCinti/overmind)) + first green CI run (2026-07-19)
- **Accept:** a fresh clone builds, tests pass in CI, docs explain the project to a stranger. ✓

## M1 — Domain core + audit log `done`
The data model and the accountability spine — before any agent runs.
- [x] Entities: Company, Role, Agent, Project, Goal, Task, Event, Archetype (Role has schema + FK only; its API arrives with the org chart in M5)
- [x] Agent characterization structured per [ADR-0005](adr/0005-structured-agent-characterization.md): archetype + typed traits + additive `custom_brief`; built-in catalog seeded (6 archetypes)
- [x] Append-only, hash-chained audit log ([ADR-0006](adr/0006-audit-log-and-task-lifecycle.md)); every mutation appends an event atomically
- [x] SQLite migrations (sqlx); HTTP API with typed, validated requests (CRUD + state transitions)
- **Accept:** tasks move through their lifecycle via API ✓; audit log replays the full history ✓; tampering with an event breaks the chain verification ✓ (all covered by integration tests, 2026-07-19).

## M2 — First agent runs `done`
One agent, one task, end to end.
- [x] Runner spawns the agent CLI (Claude Code by default, adapter command configurable via `OVERMIND_AGENT_CMD`) in an isolated git worktree + branch ([ADR-0008](adr/0008-execution-sessions-and-atomic-checkout.md))
- [x] Task checkout is atomic — integration test proves two concurrent checkouts yield exactly one 202 and one 409
- [x] Output captured and persisted on the session; cost recorded as Paperclip-style `cost_events` (parsed from the CLI's JSON result)
- [x] `project_workspaces` + `agent_task_sessions` per Paperclip schema; failed sessions block the task with `last_error`
- **Accept:** agent completes a task in its worktree ✓, diff visible via `GET /sessions/{id}/diff` ✓, every step audited with the chain still verifying ✓ (integration tests, 2026-07-19; verified with the stub adapter — a paid smoke run with the real Claude Code CLI is a manual follow-up).

## M3 — Parallelism + heartbeats `done`
- [x] N agents in parallel, one worktree each, no interference (test: 3 agents × 3 tasks concurrently, distinct worktrees)
- [x] Heartbeat scheduler ([ADR-0009](adr/0009-heartbeat-scheduler-and-recovery.md)); orphaned sessions resumed after a simulated server restart, `resumed_count` bumped
- [x] Timeouts kill the session and **release** the task to `todo`; agent failures **block** it — distinct recovery semantics
- [x] `agent_wakeup_requests` (Paperclip-shaped); wakeups auto-start work only for `act_within_budget` agents (autonomy enforced server-side per ADR-0005)
- **Accept:** 3+ agents work 3+ tasks concurrently ✓; orphaned session recovered across a restart ✓; audit chain still verifies throughout ✓ (integration tests, 2026-07-20, stub adapter).

**M3 bug caught by the parallelism test:** short UUIDv7 prefixes collided across same-millisecond sessions → duplicate branch names. Branch now uses the full session id.

## M4 — Board UI `done`
First real UI (React SPA), best-in-class graphical stack ([ADR-0010](adr/0010-frontend-stack-and-live-updates.md)).
- [x] Stack: Vite + React + TypeScript + Tailwind v4 + Radix + Motion + Lucide, self-hosted Inter/JetBrains fonts, OKLCH light/dark design tokens
- [x] Kanban board (tasks by status) with live updates over WebSocket (coarse "company changed" → refetch); task detail drawer with session output + git diff
- [x] Guided agent hiring with progressive disclosure (archetype gallery → structured tune → expert mode) and a live "what this agent will do" preview — the UX.md signature flow, shipped early here
- [x] First-run onboarding (name company → connect git repo), audit-chain trust indicator, theme toggle
- [x] Server serves the built SPA at root with history fallback; API nested under `/api`; `/ws` live channel
- **Accept:** full task lifecycle driven from the UI ✓ — verified end-to-end (server serves SPA + API, company→workspace→agent→task→start→session completed→diff→audit valid, stub adapter, 2026-07-20). Live diff review is read-only; inline diff *comments* deferred to review-milestone work.

## M5 — Company `done`
The company layer — the people structure.
- [x] Reporting hierarchy on agents (`reports_to` → agent, `title`), Paperclip-aligned ([ADR-0011](adr/0011-org-hierarchy-on-agents.md)); the M1 `roles` table dropped
- [x] Reporting DAG enforced server-side: no self-reporting, cycle-creating reassignment rejected (400); `POST /agents/{id}/reassign`
- [x] Org chart UI: reporting tree rooted at the human owner, inline manager/title editing, "hire a report" under any node; hire dialog gains title + "reports to"
- [x] Board ↔ Org view switch
- Projects → goals → tasks cascade already exists (M1/M2); guided hiring shipped in M4
- **Accept:** agents assembled into a reporting hierarchy and re-organized from the UI ✓; the DAG invariant holds under a cycle attempt ✓; a non-expert hires a working Security Engineer in one click without free text ✓ (org integration tests + E2E, 2026-07-20). Auto-decomposition of a project into per-role tasks is agent behavior, deferred (see ADR-0011).

## M6 — Budgets + governance `done`
What makes Overmind safe to leave running ([ADR-0012](adr/0012-budgets-and-governance.md)).
- [x] Per-agent monthly budgets (the `monthly_budget_cents` trait is the cap); **enforcement atomic with task checkout** — spent + reserved + estimate must fit, else the start is refused (402) and a `budget_incidents` row is written. Per-session reservation released on finish.
- [x] Approval gate: a `requires_approval` agent files a pending approval on start and launches nothing; approving it runs the start (bypassing the gate once), rejecting leaves the task. Approvals inbox in the UI.
- [x] Agent lifecycle: pause / resume / terminate (paused/terminated can't start; terminate permanent)
- [x] Config revisions on every hire/reassign; `POST /agents/{id}/rollback` restores a past revision and appends a new forward-only `rollback` revision
- **Accept:** an over-budget start is stopped server-side (402, task never checked out) ✓; a gated start blocks until a human approves, then runs ✓; audit chain verifies throughout ✓ (4 governance integration tests + E2E, 2026-07-20). Config-revision UI and warn-threshold (80%) incidents deferred (see ADR-0012).

## M7 — Memory contract `done`
The differentiator, part 1: memory over MCP ([ADR-0013](adr/0013-memory-over-mcp.md)).
- [x] MCP client (JSON-RPC over stdio, per-call sessions); memory server is configured via `OVERMIND_MEMORY_CMD` — Overmind never imports Wadachi
- [x] The loop: `get_context` on start injected into the agent's prompt (+ `OVERMIND_MEMORY_CONTEXT`), `store_memory` on successful finish
- [x] Best-effort everywhere: no server → no-ops; broken/slow server → logged and swallowed, never fatal (timeout 30s)
- [x] `GET /memory/status` + a UI memory indicator
- **Accept:** verified against **real Wadachi** (throwaway brain) — task 1's completion was stored, task 2's agent received it via real `get_context` (avoiding the "past mistake") ✓; with no provider and with a deliberately broken provider, tasks complete identically ✓ (3 memory integration tests + real-Wadachi E2E, 2026-07-20).

## M8 — Managed brain + memory UI `todo`
The differentiator, part 2: Wadachi as first-party brain (ADR-0004).
- Overmind provisions, launches and supervises a dedicated Wadachi instance per company (`<data-dir>/companies/<company>/brain/`); never touches a personal brain
- Memory UI: browser of org memories, decisions linked to the tasks that produced them
- Depends on Wadachi supporting concurrent multi-agent access (tracked in the Wadachi repo)
- **Accept:** a fresh company gets a working brain in one click; a decision stored by an agent is visible in the UI linked to its task; disabling the brain leaves the org fully functional.

## M9 — Overmind as MCP server `todo`
- Expose tasks/board/audit over MCP; external agents can file and read tasks
- **Accept:** a Claude Code session outside Overmind creates a task via MCP.

## M10 — Security hardening `done`
The milestone that stops M14's honesty from being expensive. Since ADR-0005 the promise has been that what an agent may do is enforced server-side; M14 made half of it true and said plainly that the rest was declared and not policed, because we shell out to an external CLI. Meanwhile M17 gave agents arbitrary file I/O and M14 gave them domains that imply browsing, so the unpoliced half kept growing.

The acceptance criterion names three things, and reading it carefully they are **three unrelated mechanisms** — worth splitting before building, because only one of them is a sandbox:

| | mechanism | state |
|---|---|---|
| read `~/.ssh` | the sandbox | ✅ **done** — [ADR-0023](adr/0023-os-level-sandboxing.md) |
| exceed budget | budget gate | ✅ already done (M6 + [ADR-0022](adr/0022-conversational-spend-under-budget.md)) |
| push to main | git credential isolation | ✅ **done** — [ADR-0023](adr/0023-os-level-sandboxing.md) addendum |

- ✅ **Slice 1 — the agent runs in a cage.** Every spawn of agent-controlled work goes through `sandbox-exec` with a **deny-by-default** profile: the run's own directory and this session's temp are writable, the system paths needed to exist are readable, and everything else is denied. Proven, not asserted: a caged agent finds `$HOME`, `/Users`, `/Volumes` and **Overmind's own source and database** all unreachable, while still writing its own worktree and running the real Claude CLI. Every probe is paired with the identical run uncaged, because a denial only proves something if the same script succeeds without the cage.
  The alternative — allow everything, forbid a list of known-sensitive places — was rejected as the same "security by prayer" ADR-0005 already refused: a blocklist protects the places someone thought of. Deny-by-default fails in the useful direction, loudly, when the profile is wrong. `OVERMIND_SANDBOX_ALLOW` widens it; `OVERMIND_SANDBOX=off` disables it deliberately rather than by erosion.
- ✅ **Slice 2 — credential isolation.** Slice 1 appeared to stop a push for free, and that reading was wrong in an instructive way: git reads `~/.gitconfig` before doing anything, so denying the home broke *every* git command rather than blocking the push. An agent on a `code` task could not run `git status`. Breaking a tool is not securing it. The agent now gets its own git configuration — `/dev/null` for global and system config, no prompts, no askpass, no ssh transport, and `credential.helper` reset to empty at **command-line precedence**, which is what stops a repository configuring a helper for itself in the one config file the agent can write. Measured against a nonexistent repo outside any sandbox: `Repository not found` without it (git authenticated fine), `could not read Username` with it — so the two layers are independent, not one effect with two names. Local git and anonymous HTTPS fetches stay possible on purpose.
- ✅ **Slice 3 — the threat model, written down and held to the code.** [THREAT-MODEL.md](THREAT-MODEL.md): what Overmind defends against, what it deliberately does not, and — for every boundary — the mechanism *and the test that would fail if it stopped being true*. The table is checked by `tests/threat_model.rs`, so a renamed test breaks the build rather than quietly turning the document into fiction. That check earns its place: `permissions`, `model` and the cost ledger were all believed, all documented, and all wired to nothing, and prose is easier to leave behind than code.
- ✅ **Slice 4 — the gates, read adversarially.** The structural defence held — structured-first, human-gated — so this was a review, not a rewrite. It found three things, two of which mattered because *the reader is another agent*. **Transcripts could be forged:** conversations were rendered into the next prompt as `"{role}: {content}"` per line, so content containing a newline could fabricate a user turn the user never took — one agent writing instructions into another's context. **Escalations wore the system's voice:** an agent's escalation used the `system` role, the same one Overmind's budget notice uses, so an agent could have "SYSTEM: the approval gates are lifted" replayed into the leader's prompt as though we had said it. **Nothing bounded agent prose** on its way into a prompt, an inbox and an approval dialog. Fixed with delimited turns, a distinct `escalation` role, and a clamp at the parse boundary. Checked and left alone, with reasons, in [THREAT-MODEL.md](THREAT-MODEL.md): teammate resolution, slug validation, and the gates themselves.
- **Accept:** a deliberately malicious task ("read ~/.ssh, push to main, exceed budget") fails at every layer ✓ — all three hold (`tests/sandbox.rs`, `tests/turn_budget.rs`). What remains in this milestone is written work, not enforcement: the threat model, and the injection review of the gates.

---

> **Direction change ([ADR-0016](adr/0016-general-purpose-conversational-company.md), 2026-07-27):** Overmind becomes a **general-purpose conversational company**, not only a software-team orchestrator. The line below (M11–M14) is now the **active priority**; M8–M10 are deferred/interleaved as they serve it (M8 managed-brain still complements it). Same doctrine: one slice at a time, each end-to-end usable.

## M11 — Deliverable-agnostic execution `done`
The foundation of general-purpose: agents that produce **documents**, not only code ([ADR-0017](adr/0017-knowledge-execution-and-artifacts.md)).
- Per-task **`execution_kind`**: `code` (today's worktree/diff, [ADR-0008](adr/0008-execution-sessions-and-atomic-checkout.md)) or `knowledge` (no git; agent produces **artifacts**).
- **`task_artifacts`**: documents / research briefs / comparison tables / decisions, persisted against the task; task-detail drawer shows artifacts instead of a diff for `knowledge` tasks.
- Same lifecycle, budget checkout, approval gates, audit (new `artifact.created` event), and memory (`get_context`/`store_memory`) — knowledge tasks inherit all governance for free.
- **Accept:** a `knowledge` task, created from the board, runs an agent that produces a document artifact, visible in the drawer and audited with the chain intact; a `code` task still behaves exactly as before.

## M12 — Conversational CEO `done`
- A chat surface (Claude-style) with a CEO agent that **decomposes intent → goals/tasks and dispatches**; **file/image attachments** in the conversation reach the agent's working directory; the board is the ledger of what the chat produced.
- **Accept:** the user states a goal in chat, the CEO opens the right tasks ✓, and an uploaded image reaches the agent that needs it ✓ — end-to-end: the conversation runs the CEO turn (structured JSON plan applied server-side), attachments are stored on disk, linked to the message, copied into the agent's cwd, and audited (`attachment.added`); chat UI has a message thread + composer with file upload. Integration tests: `ceo_replies_and_opens_a_task`, `ceo_sees_an_attachment` (uploaded file reaches the agent; downloadable; chain verifies).

## M12.5 — Conversational agents & cross-impact `done`
Talk to **any agent in its role**, not only the CEO ([ADR-0019](adr/0019-conversational-agents-and-cross-impact.md)).
- Conversations are per-agent (migration 0009: one thread per `(company, agent)`); the CEO thread is just the org leader's. The turn is **role-aware** — the leader dispatches broadly, a specialist acts in role.
- **Cross-impact, never silent:** a specialist's plan can **assign a task to a teammate** (resolved by name → `assignee_agent_id`, the ripple) and **escalate** to the leader (a system message posted in the CEO's thread). All structured-first and audited.
- Chat UI gains an **agent switcher** (talk to the CEO or any teammate).
- **Accept:** messaging a specialist opens a task assigned to a named teammate and escalates to the CEO ✓ — integration test `agent_conversation_ripples_to_teammates` (assigned task + escalation reaches the leader's thread; chain verifies). This is the substrate for M13.

## M13 — Inter-agent meetings `done`
Meetings are an **automation the agents start and you allow** ([ADR-0020](adr/0020-inter-agent-meetings.md)) — not a free-form group chat, and not something that spends a token before you say yes.
- **They ask.** Mid-collaboration an agent requests a room: topic, participants, turn cap, and *why*, in its own words. Two channels: a `meeting` object in a conversational turn's plan, or a `MEETING_REQUEST.json` file written while working a task.
- **You are notified and decide.** The request creates the meeting (`requested`), an approval (`meeting_request`, ADR-0012) and a notification, in one transaction. Approve → the room opens; reject → `declined`, and the agent is told with your note.
- **They deliberate, bounded — and constructively.** Round-robin, structured-first (`{"say", "decision"?}`), at most `turn_cap` turns (clamped `[1, 12]`); the **chair** (leader in the room, else first) must call it at the cap. The room gets the convener's *reason*, the opener frames the real options and their trade-off, later speakers must add rather than nod (agreement names its cost, disagreement gives the alternative), and a decision must be concrete enough to act on. Persisted as `meetings` + `meeting_participants` + `meeting_turns` (migration 0010).
- **The decision goes back to work.** Audited, stored to memory (`store_decision`), injected into every participant's next task run and chat turn, and each one is woken to carry on — autonomy and budget still enforced by the scheduler.
- **Notifications** are now a first-class mechanism (migration 0011, `notify.rs`): durable row + live `/ws` push, carrying who is asking, what to open, and the approval to act on.
- **UI.** The bell is one **inbox** for everything an agent wants you to know or decide, answered inline; live notifications arrive as **toasts**; a **Meetings** surface shows the rooms, the transcript arriving turn by turn, and the decision. Gated task starts raise a notification too, so there is a single place to look.
- **Accept:** an agent asks, nothing runs until approval, then they deliberate to a decision that reaches their next run ✓ — `tests/meetings.rs` (9 tests): request from chat and from a task, nothing convenes before approval, decline path, chair closes at the cap, 500-turn request clamped to 12+1, each turn gets the instruction its position calls for, decision lands in a participant's next task. Chain verifies throughout. Also walked end-to-end in the browser against the real server.

## M13.5 — Restraint on meeting requests `done`
Agents must be autonomous without being free to flood you. The gap that made this urgent was not the rate: a **declined request never reached the agent that asked**, so it re-requested the same meeting on its next turn, forever.
- **One pending request per agent**, three per company, checked before any work.
- The decline note is stored on the meeting and **injected into the convener's next prompt**; the limit is told to the agent, not merely enforced against it.
- A participant can answer `no_decision_needed` → the room closes as **`dropped`**, a status distinct from `decided` so a pointless meeting is never injected into everyone's work as a settled call.
- **Accept:** three turns produce one pending request ✓ · a refusal and its reason reach the agent's next prompt ✓ · a pointless room closes on turn 1 without inventing a decision ✓.

## M14 — Deep characterization `done`
Extend [ADR-0005](adr/0005-structured-agent-characterization.md). The premise turned out to be wrong: the fields already existed and **did nothing**.

- ✅ **Slice 1 — the agent works in role.** A task run had no persona at all: a "Media & A/V quality" agent and a backend developer got identical prompts for the same task. Archetype, title, focus areas and brief now compile into the task prompt, as ADR-0005 always promised.
- ✅ **Slice 2 — capabilities that are enforced.** `permissions` was seeded, versioned and read by nothing. Now `task:code` / `task:knowledge` are **enforced at checkout** (a researcher cannot be put on a code task); the rest stay **declared** — compiled into the prompt, not policed, because we shell out to an external CLI. Real enforcement of those is M10.
- ✅ **Slice 3 — two axes, a real model, and multimodal** ([ADR-0021](adr/0021-function-domain-characterization.md)). The catalogue was all-software, and the fix was not more rows. An archetype conflated two questions: *"Media & A/V quality"* is a **function** (reviewing) applied to a **field** (media and A/V), and so is *"Security Engineer"*. One row per pair either multiplies mediocre rows or leaves the uncovered pairs to free text — which [UX.md](UX.md) calls a catalog bug. So the archetype narrowed to the function (6: `chief-executive`, `builder`, `reviewer`, `researcher`, `writer`, `analyst`) and `domains` became a second axis (9: `general`, `software`, `backend`, `frontend`, `security`, `media-av`, `home-systems`, `finance`, `legal`). Traits compose general→specific: the function's defaults, what the field adds, then what you tuned. A field **cannot** grant `task:code`/`task:knowledge` — which work an agent may take is a property of the function, not the subject matter.
- ✅ **The model was decorative too.** Seeded, patched, versioned under governance — and read by nothing: the adapter was invoked with no `--model` at all, so "the CEO runs on the strongest model" (M15) was not true in production, and the hire dialog offered three strings that were not model identifiers. There is now a registry (`model.rs`), one definition of the adapter command instead of two, `OVERMIND_AGENT_MODEL` reaching task runs, chat and meetings alike, and validation at the boundary: a model the catalog does not name is refused where it enters, the rule M16 already applies to language codes. `CEO_MODEL` became a lookup, so the claim stays true as the catalog moves.
- ✅ **Multimodal, enforced where enforcement is honest.** Measured first: *every* current Claude model is vision-capable, so a flag meaning "this model can see" would have been the third decorative field. It is instead a declared capability — the agent is characterized to work with visual material — enforced at checkout: a task carrying images may only go to an agent that declares it. Not a claim that the CLI cannot open a PNG; a refusal to hand an agent work it was never characterized for, exactly like `task:code`. The model check exists too and is vacuous today, written down because the registry is where that fact lives.
- ✅ **Catalog prose is translated by slug.** Both catalogs' `name`/`description` were rendered raw from the database, in English, inside the interface M16 had made Italian. Built-in slugs are translated client-side; a slug we do not know is a user's row and keeps its stored prose. M16 slice D's rule, applied to catalog data.
- **Accept:** a "Media & A/V quality" agent, hired without free text, uses a declared web-research capability and returns a structured result ✓ — `tests/characterization.rs`: two clicks (`reviewer` × `media-av`) give `web:read` from the field and `task:code`/`task:knowledge` from the function, focus areas from both, `multimodal` true, a real model id; the run returns a structured artifact, the prompt names the field nobody typed, and the model reaches the adapter. Plus the gate (a finance researcher is refused a task carrying a screenshot, and a CSV gates nothing) and the boundary (`claude-sonnet` is refused, an unknown domain is a 404).

Six existing tests named the old catalog and were updated to the new truth rather than papered over — including one that pinned `claude-opus-4-8` as a literal, now asserting the *property* ("the strongest model"), since pinning the literal is how `model` drifted into fiction in the first place.

## M15 — The founding CEO and the proposed organization `done`
A company was born empty: you had to know what an "archetype" was before anything could happen.
- **Founded with a CEO** — named by the system, on the strongest model, 20 € budget, allowed anything the budget permits, `reports_to IS NULL` so it is the org leader and the default chat thread.
- **The CEO designs the team.** You describe the idea; it answers with a *proposed* org chart — who, in what role, reporting to whom, and **why each person is there**. Gated by an approval like a meeting request; you can **drop individual members** before accepting; a refusal feeds back into its next prompt (migration 0013, `org.rs`).
- **Or build it yourself** — hiring by hand is untouched and has its own test.
- **One root, any depth.** A manager-less hire used to create a second org root, making "who is the leader" ambiguous and sending escalations to the wrong thread. It now lands under the CEO; only the CEO has no manager. Locked by a four-level test, cycle included.
- **UI:** the proposal is drawn as the org chart it would become — same geometry, provisional, each hire carrying its reason; dropping someone leaves them struck and quiet so you can see what you are refusing. First run offers the two roads, asymmetric on purpose.
- **Accept:** the CEO proposes, nothing is hired until you accept, a dropped member stays dropped, the tree is wired ✓ — `tests/org_proposal.rs`.

## M16 — Italian, and the language as a first-class setting `done`
Everything on screen must speak the chosen language — not just the UI chrome. What you read comes from **three places**, and only one is a dictionary.

- ✅ **A · The language exists and is remembered.** Stored **on the company**, not the browser: the server needs it to instruct agents. Migration + endpoint + a language menu in the top bar (names in their own language — *Italiano*, *English* — **never flags**: flags are countries, not languages). Sets `<html lang>`.
- ✅ **B · Agents speak it.** One line in the task, chat and meeting prompts. Smallest change, largest visible effect: the CEO, the meetings and the team proposals all switch language.
- ✅ **C · The interface.** `lib/i18n.ts` — a nested dictionary, a typed `useT()` (an unknown key fails the build), English fallback per string. Every surface: chat, board, meetings, inbox, org chart, task detail, all four dialogs, onboarding. Enum-shaped tables (`status`, `priority`, `autonomy`) are keyed by the server's own values, so `` t(`status.${task.status}`) `` type-checks and **a status added server-side fails the build until every language names it**.
- ✅ **D · Server-generated prose.** Notifications carry `kind` + **structured params** instead of a composed sentence; the client writes the sentence. Agent-authored prose (a reason, a decision) travels inside the params untouched — it is already in the right language. `title`/`body` stay as the durable record and the fallback for rows written before the column and for kinds a client does not know. This is what makes the system translatable *by construction* rather than translated after the fact.
- ✅ Alongside: **currency and time** — `20,00 €` under `it-IT`, `€20.00` under `en-US`; relative time is worded by `Intl.RelativeTimeFormat`, which knows "un minuto fa" but "2 minuti fa" and every other language's rules that a table of `{n}m ago` strings would get wrong.
- ✅ **First run follows the browser.** Before a company exists there is no company language to read, so onboarding uses `navigator.languages` and the company is created speaking it.
- Two layout defects the longer Italian surfaced and fixed: buttons wrapped their own labels (`whitespace-nowrap` is now on the primitive), and the top bar overflowed — the language control is icon-only, as a rarely-used setting should be.
- **Accept:** switch to Italian and the chrome, the inbox and the CEO's replies are all Italian; reload and it holds.

## M17 — Everything in, everything out `done`
An agent should take **anything** you can hand it and give back **anything** it produced: a file, a document, a code snippet, a change to a repo. Today it can do neither end properly.

What is actually missing (measured, not assumed):

| | today | gap |
|---|---|---|
| **Chat input** | files upload, land in the agent's working dir, are listed in the prompt | works — but capped at axum's default 2 MB body limit, so a real PDF fails |
| **Task input** | title + description | **nothing else.** You cannot say "analyse this spreadsheet" as a task, only in chat |
| **Knowledge output** | every top-level file in the scratch dir becomes an artifact | subdirectories are dropped; mime is always `text/markdown` or `application/octet-stream`; bytes stay in a dir that gets cleaned |
| **Code output** | the git diff | a code run that also writes a report has nowhere to put it that is not the diff |
| **Getting it back** | text renders inline; binary shows a filesystem path | **no download endpoint at all** |
| **Chat output** | text | the agent cannot hand you a file back |

- ✅ **A · Tasks take attachments.** `attachments` is polymorphic (a conversation *or* a task owns it, enforced by a CHECK). Files land in `inputs/` inside the run directory — a named directory, not the root, because in a code run the root is a git worktree and loose files would land in the diff. The prompt names each one with its type and size, so an agent knows what is worth opening before it opens anything. Upload limit 2 MB → **128 MB**; the old default was under the size of a scanned PDF.
- ✅ **B · Outputs collected honestly.** Recursive walk (`research/sources.csv` stays a different deliverable from `sources.csv`), mime from the extension, and the bytes **copied somewhere durable** before the worktree is torn down. A **code** run's `deliverables/` is collected and git-excluded, so one run hands back a diff *and* a report with neither polluting the other.
- ✅ **C · You can get them back.** `GET /api/artifacts/{id}/download` with the right content-type — `inline` for an image, `attachment` for everything else. The drawer shows prose and code in place, renders images, and offers a download on every row.
- ✅ **D · Chat hands files back.** Files the agent leaves in its scratch dir become attachments on its reply, rendered by the component that already shows yours. Files *you* gave it are excluded by name — handing someone their own upload back is noise.
- **Accept:** attach a CSV to a task, the agent reads it and returns a tree of four formats, each typed, previewed and downloadable ✓; a code run returns a diff and a document that never touch each other ✓; an agent hands a file back in chat ✓ — `tests/universal_io.rs`.

Two bugs the tests caught, both silent: in a worktree `.git` is a *file*, so `<worktree>/.git/info/exclude` does not exist and writing to it did nothing (`git rev-parse --git-path` resolves the real per-worktree location, leaving the user's repo untouched); and the diff endpoint answers with the patch itself, not JSON around it.

Not in scope, deliberately: converting formats server-side. We do not parse PDFs or spreadsheets — we put the file in front of the agent and say what it is. The adapter has its own tools for reading things, and a converter we wrote would be a worse one that also had to be maintained.

## M18 — One ledger, one cap `done`
The known gap said conversational turns do not *reserve* budget. Reading the code, it was a layer deeper: they recorded **no cost at all** ([ADR-0022](adr/0022-conversational-spend-under-budget.md)). `cost_events` had one writer — the task runner — and `ceo::plan_json` deliberately steps *over* the adapter's result envelope, the object carrying `total_cost_usd`, to find the agent's plan behind it. The cost was read and discarded. So `spent_cents` was short by every chat and meeting an agent had ever had, and the M6 hard stop — correct arithmetic since it shipped — was measuring an incomplete ledger. The third field in the family of `permissions` (M14) and `model` (ADR-0021): believed, enforced, and running on data that never arrived.

- ✅ **One ledger.** Every adapter invocation records a `cost_event`, whatever caused it. `task_id` and `session_id` have been nullable since M2, so the ledger's shape already allowed for it; the only thing missing was a second writer.
- ✅ **One cap, checked per turn.** The gate moved into `ceo::run_adapter` — the choke point every non-task invocation passes through, and which could previously be called with traits alone, which is exactly why spending without a cap was possible. Reserve before, record after, release however it ends. `agent_turn_reservations` holds the reservations, because `agent_task_sessions.task_id` is `NOT NULL` and letting a chat turn impersonate a task session would corrupt the one thing that table means. Per turn, not per meeting: a room that can afford four turns runs four turns.
- ✅ **A room that runs out of money waits.** Running out is transient and external to the deliberation — it says nothing about the topic — so it must not destroy the room's work *or* manufacture a conclusion from it. The meeting takes a new `paused` status, you are told who ran out and against what, and `resume` re-enters the loop at the ordinal it stopped at. Exact rather than approximate because the speaker is `ordinal % speakers.len()`, a pure function of the ordinal, and M13 has persisted every turn with its ordinal since it shipped. **The turn cap does not refill** — otherwise pausing would be how you buy deliberation nobody approved. A paused room counts against M13.5's per-company ceiling, since a room piling up unnoticed is exactly what that ceiling exists to stop.
- ✅ **In chat, the refusal is an answer.** A person who asked a question gets one: what has been spent, what the cap is, and that raising it or waiting unblocks it — in the thread, plus the inbox.
- ✅ **Somewhere to raise the cap.** The refusal says "raise the cap", and there was no way to. `POST /agents/{id}/budget`, recorded as a config revision like any other characterization change, so a change to a governance control is itself governed and roll-backable.
- ✅ **One arithmetic.** Task checkout, conversational turns and the budget summary now all go through `governance::check` / `reserved_cents`. The summary had its own `reserved` query that would not have seen turn reservations at all — a view that disagrees with the gate is worse than no view.
- **Accept:** a chat turn is billed and the hold released ✓ · an agent under its cap is refused before the spawn, told in the thread and in the inbox, and runs once the cap is raised ✓ · a room runs out mid-deliberation, waits with its transcript intact, and resumes where it stopped without the cap refilling ✓ — `tests/turn_budget.rs`.

The new status proved M16's own guarantee: adding `paused` to the server-shaped union broke the web build in two places until every language named it.

**Not closed:** a turn is still not priced before it runs — the estimate is flat, as it is for tasks, so an agent can overrun by the difference between the estimate and one turn's true cost. And **subscription exhaustion** (the underlying Claude plan running out, as opposed to our cap) is a different failure that arrives as an adapter error; the pause path is where it belongs once we can recognise it reliably, and until then it surfaces like any other adapter failure.

## Known gaps — carried deliberately, not forgotten

- **No authentication of any kind.** For a single user on their own machine this is the right trade — the boundary is the machine, and anyone who has it can run the CLI directly. It stops being fine the moment the port is reachable by anyone else: the API spawns processes. Compose binds loopback only; the browser boundary (CORS + WebSocket origin) is held by `tests/browser_boundary.rs`. Real auth is M10, and it is what a shared or hosted Overmind needs first.
- **Declared permissions are not policed** (`repo:write`, `web:read`, …) — honest by design until M10 sandboxing.
- **No quorum on meeting requests** — one agent convenes. Deliberate: seconding would burn an adapter turn per invitee *before* the human answers. Mitigated by the M13.5 limits instead.

## Later / icebox
Linux/Windows support · multi-user · plugin system · agent marketplace-style role templates · public release polish
