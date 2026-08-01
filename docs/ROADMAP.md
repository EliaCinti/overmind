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

## M10 — Security hardening `todo`
- OS-level sandboxing of runners (macOS `sandbox-exec` first); secrets isolation; threat-model doc; prompt-injection review of every gate
- **Accept:** a deliberately malicious task ("read ~/.ssh, push to main, exceed budget") fails at every layer.

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

## M14 — Deep characterization `in progress`
Extend [ADR-0005](adr/0005-structured-agent-characterization.md). The premise turned out to be wrong: the fields already existed and **did nothing**.

- ✅ **Slice 1 — the agent works in role.** A task run had no persona at all: a "Media & A/V quality" agent and a backend developer got identical prompts for the same task. Archetype, title, focus areas and brief now compile into the task prompt, as ADR-0005 always promised.
- ✅ **Slice 2 — capabilities that are enforced.** `permissions` was seeded, versioned and read by nothing. Now `task:code` / `task:knowledge` are **enforced at checkout** (a researcher cannot be put on a code task); the rest stay **declared** — compiled into the prompt, not policed, because we shell out to an external CLI. Real enforcement of those is M10.
- ☐ **Slice 3 — domain archetypes + multimodal.** The catalogue is still all-software; a "Media & A/V quality" agent only exists as a title over a `researcher`. Add domain archetypes and a multimodal flag.
- **Accept:** a "Media & A/V quality" agent, hired without free text, uses a declared web-research capability and returns a structured result.

## M15 — The founding CEO and the proposed organization `done`
A company was born empty: you had to know what an "archetype" was before anything could happen.
- **Founded with a CEO** — named by the system, on the strongest model, 20 € budget, allowed anything the budget permits, `reports_to IS NULL` so it is the org leader and the default chat thread.
- **The CEO designs the team.** You describe the idea; it answers with a *proposed* org chart — who, in what role, reporting to whom, and **why each person is there**. Gated by an approval like a meeting request; you can **drop individual members** before accepting; a refusal feeds back into its next prompt (migration 0013, `org.rs`).
- **Or build it yourself** — hiring by hand is untouched and has its own test.
- **One root, any depth.** A manager-less hire used to create a second org root, making "who is the leader" ambiguous and sending escalations to the wrong thread. It now lands under the CEO; only the CEO has no manager. Locked by a four-level test, cycle included.
- **UI:** the proposal is drawn as the org chart it would become — same geometry, provisional, each hire carrying its reason; dropping someone leaves them struck and quiet so you can see what you are refusing. First run offers the two roads, asymmetric on purpose.
- **Accept:** the CEO proposes, nothing is hired until you accept, a dropped member stays dropped, the tree is wired ✓ — `tests/org_proposal.rs`.

## M16 — Italian, and the language as a first-class setting `todo`
Everything on screen must speak the chosen language — not just the UI chrome. What you read comes from **three places**, and only one is a dictionary.

- ☐ **A · The language exists and is remembered.** Stored **on the company**, not the browser: the server needs it to instruct agents. Migration + endpoint + a language menu in the top bar (names in their own language — *Italiano*, *English* — **never flags**: flags are countries, not languages). Sets `<html lang>`.
- ☐ **B · Agents speak it.** One line in the task, chat and meeting prompts. Smallest change, largest visible effect: the CEO, the meetings and the team proposals all switch language.
- ☐ **C · The interface.** `lib/i18n.ts` — a nested dictionary and a `useT()` hook, no library for ~250 strings. The long, mechanical slice: ~15 components.
- ☐ **D · Server-generated prose.** Notifications carry a `kind` + **structured params** instead of a composed sentence; the UI writes the sentence in the right language. Old rows keep `title`/`body` as a fallback. This is what makes the system translatable *by construction* rather than translated after the fact.
- Alongside: **currency and dates**. Budgets render as `$20.00` while the product is priced in euro, and dates are US-formatted.
- **Accept:** switch to Italian and the chrome, the inbox and the CEO's replies are all Italian; reload and it holds.

## Known gaps — carried deliberately, not forgotten

- **Chat and meeting turns do not reserve budget.** Task checkout has enforced budget reservation since M6; conversational turns (M12) and meeting turns (M13) do not. A meeting can spend up to 13 adapter runs without the agent's monthly cap being consulted. With the CEO now proposing teams and teams holding meetings, **this is the most pressing hole.**
- **No authentication of any kind.** Fine while bound to loopback; anything beyond that is unauthenticated access to an API that spawns processes. See M10 and `docs/adr/0014-docker-deployment.md`.
- **Declared permissions are not policed** (`repo:write`, `web:read`, …) — honest by design until M10 sandboxing.
- **No quorum on meeting requests** — one agent convenes. Deliberate: seconding would burn an adapter turn per invitee *before* the human answers. Mitigated by the M13.5 limits instead.

## Later / icebox
Linux/Windows support · multi-user · plugin system · agent marketplace-style role templates · public release polish
