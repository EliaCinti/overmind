# Threat model

> What Overmind defends against, what it does not, and which test holds each
> claim. Written for M10 slice 3, when the boundary had stopped fitting in
> comments.

Every claim below names the mechanism that enforces it and the test that would
fail if it stopped being true. A security document nobody can check is a
security document that quietly becomes fiction — the same failure this project
has now found three times in its own code (`permissions`, `model`, and the cost
ledger), so it is worth not repeating in prose.

## The shape of the thing

Overmind is a Rust server on `localhost` with a web UI, run by one person on
their own machine. It stores everything in a local SQLite file. To do its work
it **spawns processes**: an agent adapter (the Claude Code CLI by default),
`git`, and optionally an MCP memory server. Those processes reach the network.

Two facts follow, and most of this document follows from them:

1. **The API can start a process on your computer.** Anything that can reach the
   API can, in effect, run code as you.
2. **The agent is not a program we wrote.** It is an LLM driving a third-party
   CLI, acting on text that may include material we did not author.

## What is worth protecting

| | why |
|---|---|
| Credentials — `~/.ssh`, keychain, tokens, cloud config | the obvious prize, and the one an agent has no business touching |
| The rest of the machine — documents, browser profile, other projects | an agent has a job; the job is not "your home directory" |
| Overmind's own state — `overmind.sqlite`, the audit chain | an agent that can edit the ledger can hide what it did |
| Money — the monthly caps | an agent in a loop is an expensive agent |
| Your repositories — what gets pushed, and by whom | a wrong commit is recoverable; a wrong push to `main` is a conversation |

## Who this defends against

**A hostile web page.** Real, and the least obvious. Running "only on localhost"
is not a boundary: every page you have open can talk to `127.0.0.1`, and this
API spawns processes. Addressed in M13's security fix — CORS exists only when
there is no built frontend, and `/ws` judges `Origin` itself because WebSockets
bypass CORS entirely.

**Prompt injection.** The primary realistic threat. M17 let you hand an agent
arbitrary files, and M14 gave agents fields that imply reading the web. Text
inside a PDF, a scraped page or a repository file can try to instruct the agent.
We do not attempt to detect it — that is a losing game — so the defence is that
a successfully injected agent still cannot reach anything.

**An agent that is simply wrong.** Misreads the task, deletes the wrong thing,
loops. Indistinguishable from the above in terms of what contains it, which is
convenient: the same mechanisms cover both.

## Who this does *not* defend against

Stated plainly, because a boundary you have not drawn is not a boundary you can
rely on.

- **Anyone with the machine.** There is no authentication of any kind. The
  boundary is the physical machine: someone sitting at it can run the CLI
  directly and does not need Overmind's API to do harm. This is the correct
  trade for a single-user local tool and the wrong one the moment the port is
  reachable by anyone else — which is why Compose publishes on loopback only.
- **A malicious adapter.** If the Claude Code CLI were hostile, the sandbox
  would limit what it reaches but we would still be handing it the task and the
  worktree. We do not verify the binary.
- **A malicious MCP memory server.** It is a command *you* configured
  (`OVERMIND_MEMORY_CMD`) and it runs outside the cage, like `git`.
- **The network.** The cage cannot close it — reaching the API is the job — so
  an agent can talk to any host it can name. What it cannot do is talk to them
  *as you*: see credentials, below.
- **Someone who edits the database directly.** The audit chain is
  **tamper-evident**, not tamper-proof: `GET /api/audit/verify` detects a
  modified event, it does not prevent one.

## The boundaries, and what holds them

| Claim | Mechanism | Held by |
|---|---|---|
| A web page you visit cannot drive the API | CORS only in dev; explicit `Origin` guard on `/ws` | `browser_boundary.rs` — `a_hostile_page_cannot_reach_the_api`, `the_live_socket_refuses_a_foreign_origin` |
| A non-browser client (curl, tests, MCP) still works | absent `Origin` is not a browser | `browser_boundary.rs` — `a_non_browser_client_still_works` |
| An agent cannot read your home, other volumes, or Overmind's own source and database | `sandbox-exec`, deny-by-default ([ADR-0023](adr/0023-os-level-sandboxing.md)) | `sandbox.rs` — `a_caged_agent_cannot_reach_the_machine_it_runs_on`, paired with `the_same_agent_uncaged_reaches_everything` |
| An agent cannot write outside its own run directory | same | same |
| An agent has no credentials to push with, even if its repository configures a helper | `GIT_CONFIG_*` at command-line precedence; no ssh, no prompt, no askpass | `sandbox.rs` — `git_still_works_and_has_no_credentials_to_push_with` |
| …while git itself still works locally | `GIT_CONFIG_GLOBAL=/dev/null` rather than denying `~/.gitconfig` | same |
| An agent cannot exceed its monthly cap, in tasks *or* conversation | atomic checkout gate; per-turn gate ([ADR-0012](adr/0012-budgets-and-governance.md), [ADR-0022](adr/0022-conversational-spend-under-budget.md)) | `governance.rs` — `start_is_stopped_when_over_budget`; `turn_budget.rs` — `an_agent_out_of_budget_is_refused_before_it_spends` |
| A gated agent starts nothing until a human approves | approval gate | `governance.rs` — `approval_gate_blocks_until_approved` |
| A paused or terminated agent cannot work | status check at checkout | `governance.rs` — `paused_and_terminated_agents_cannot_start` |
| An agent cannot take work it is not characterized for | capability gate ([ADR-0005](adr/0005-structured-agent-characterization.md), M14) | `meetings.rs` — `an_agent_is_refused_work_it_is_not_characterized_for` |
| An agent cannot be handed visual material it was not characterized to judge | multimodal gate ([ADR-0021](adr/0021-function-domain-characterization.md)) | `characterization.rs` — `an_agent_is_refused_material_it_was_not_characterized_to_look_at` |
| An agent cannot flood you with meeting requests | one pending per agent, three per company (M13.5) | `meetings.rs` — `an_agent_may_keep_only_one_request_waiting_on_you` |
| Editing the audit log is detectable | SHA-256 hash chain ([ADR-0006](adr/0006-audit-log-and-task-lifecycle.md)) | `api.rs` — `tampering_with_an_event_breaks_the_chain` |

## Where agent-authored input enters

Every gate above that an agent can *ask* something of takes text the agent
wrote. That surface has never been reviewed with an adversarial eye, and it is
the subject of M10's remaining slice:

| Entry point | Parsed by | Becomes |
|---|---|---|
| The chat/meeting plan | `ceo::plan_json` | tasks, assignments, escalations |
| `meeting` in a plan, or `MEETING_REQUEST.json` written during a task | `meeting::Request::from_json` | a room, an approval, a notification |
| `team` in a plan | `org::Proposal::from_json` | proposed hires, an approval |
| Archetype and domain slugs in a proposal | `org.rs` validation | who gets hired, with what capabilities |
| Artifacts and reply attachments | `files.rs` | files you are shown and may open |

The structural defence is already in place and is why this is a review rather
than a rewrite: **everything is structured-first**. An agent emits validated
JSON, not prose that mutates state, and every consequential action is gated on a
human decision. The questions the review must answer are whether the *content*
of those fields can mislead the human who approves them, and whether any
validation can be walked past.

## Keeping this true

- **When a gate is added, add a row.** A gate with no row here is a gate nobody
  is tracking.
- **When a claim's test is deleted or renamed**, the row is wrong — the table
  points at test names on purpose so this is noticeable.
- **Re-read this at every milestone that widens what agents can do.** M17
  (arbitrary file I/O) and M14 (domains implying web research) each widened the
  surface without anyone re-reading the boundary, which is how the unpoliced
  half grew expensive enough to force M10.
