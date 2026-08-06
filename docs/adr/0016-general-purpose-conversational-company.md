# ADR-0016: General-purpose conversational company

- **Date:** 2026-07-27
- **Status:** accepted

## Context

M0–M7 built a **software-team orchestrator**: Company → Project → Goal → Task on a Kanban board, where an agent claims a task and executes it in an isolated **git worktree**, producing a **code diff** ([ADR-0008](0008-execution-sessions-and-atomic-checkout.md)), governed by budgets and approval gates ([ADR-0012](0012-budgets-and-governance.md)), with organizational memory over MCP ([ADR-0013](0013-memory-over-mcp.md)). The roadmap icebox lists *"non-coding agent workflows (research, content)"* as deferred — the current product is, by design and by execution model, about **code**.

New product intent from the owner: use Overmind for **real-world, non-code projects** (first concrete one: designing and building a home cinema), and more broadly turn Overmind into a **general-purpose "AI company you talk to."** Concretely:

- create a team and **characterize each agent** deeply (domain roles: sound engineer, acoustics/measurements, purchasing, product research, video…);
- **talk to a CEO in a chat** (Claude-chat style) with **file and image attachments**;
- the CEO **decomposes intent and dispatches** work to the team;
- agents **deliberate among themselves** ("meetings") to reach decisions;
- everything runs **locally**, self-hosted, with a **carefully designed** UI, and the owner must be able to **understand and modify** it.

This is a **direction-setting** ADR. It commits the product direction and the architectural shifts; implementation lands across new milestones (M11+), one end-to-end slice at a time per the project doctrine.

## Decision

Evolve Overmind from a code-delivery orchestrator into a **general-purpose conversational company**. Today's software behavior becomes **one specialization**, not the whole product. Five axes, in dependency order.

1. **Deliverable-agnostic execution (the foundation).** Generalize execution from "git worktree → code diff" ([ADR-0008](0008-execution-sessions-and-atomic-checkout.md)) to a pluggable **execution kind** per task. Two kinds to start:
   - `code` — today's worktree/diff flow, unchanged;
   - `knowledge` — the agent produces **artifacts** (documents, research briefs, decisions, comparison tables) persisted against the task; **no git, no diff**.
   Same task lifecycle, audit, and governance; only *what an agent produces and where it is stored* differs. Without this, none of the rest can actually deliver — it is the true blocker.

2. **Conversational CEO layer.** A first-class **conversation** with a designated **CEO agent** (Claude-chat-style surface). The user states intent; the CEO **decomposes into goals/tasks and dispatches** to the team. The board is **not** replaced — it becomes the structured ledger of what the conversation produced. Auto-decomposition (explicitly deferred in M5) becomes a core CEO capability. Every dispatch is still an audited task creation.

3. **Inter-agent deliberation ("meetings").** Agents exchange **messages** inside a **bounded, orchestrated** deliberation to reach a decision (e.g., sound + acoustics + purchasing converge on a spec). A meeting has a goal, named participants, a **turn/round cap**, and a **budget**; it terminates with a **recorded decision** stored to memory via Wadachi. This is a structured, cost-capped protocol — deliberately **not** free-form group chat, to prevent loops and runaway cost.

4. **Attachments & multimodal input.** The conversation and `knowledge` tasks accept **files and images**; attachments are stored and referenced; agents that consume them declare and use **vision-capable** models.

5. **Deeper, honest characterization.** Extend [ADR-0005](0005-structured-agent-characterization.md)'s structured traits with a **domain brief**, declared **tools/capabilities** an agent may use (e.g. web research, spreadsheet output, vision), and a multimodal flag — still **structured-first**, enforced by the server, never just prompt text.

**Unchanged, cross-cutting:** Paperclip vocabulary (Company/Project/Goal/Task, [ADR-0007](0007-paperclip-vocabulary-alignment.md)); structured-first characterization; memory over MCP (Wadachi), now also the substrate for meeting decisions and cross-agent context; append-only hash-chained audit (new event kinds for conversation/message/meeting/decision/artifact/attachment); budgets + approval gates, which now also cover conversations and meetings. Local-first and self-hosted. UI stays the [ADR-0010](0010-frontend-stack-and-live-updates.md) stack (Vite + React + Tailwind + Radix + Motion); the chat is a new surface, designed with the team's design skills.

## Alternatives considered

- **Keep Overmind code-only; run non-code projects on a separate lightweight tool.** Cleanest scope, but abandons the most valuable direction and the owner's real use case. Rejected *as the product direction* — though a lightweight setup may still run the home cinema **in the interim**, feeding requirements back here.
- **Bolt a chat onto the current board without generalizing execution.** Lets you "talk to a CEO," but tasks still expect code/worktrees, so agents cannot actually deliver non-code work. A chat without the execution kind is a demo, not a product. Rejected.
- **Free-form agent group chat.** Simplest to picture, but unbounded multi-agent conversation loops and burns budget. Rejected in favour of the bounded "meeting" protocol.
- **Replace the board with the chat.** The board is Overmind's accountability spine (status, audit, governance). Keep both: **chat drives, board is the ledger.** Rejected replacement.

## Consequences

- **Big scope, sequenced across milestones** (each end-to-end usable per doctrine):
  - **M11** — deliverable-agnostic execution: the `knowledge` kind + task artifacts.
  - **M12** — CEO conversation: decompose/dispatch + attachments in chat.
  - **M13** — inter-agent meetings + recorded decisions.
  - **M14** — richer characterization: domain brief + declared tools + multimodal.
  - The two riskiest designs — the **artifact model** and the **bounded-meeting protocol** — get their own ADRs when M11/M13 start.
- **First thin slice** (early M11→M12): user chats with a CEO → CEO opens **one** `knowledge` task → **one** domain agent produces **one** document → visible on the board + audited. No git, no meetings yet.
- **The home cinema is the first real "company"** on this line and the acceptance scenario that keeps the work honest.
- **Governance extends to conversation:** budgets/approval/audit now cover chat turns and meetings; a meeting cost-cap is a new governance surface, and the M10 prompt-injection review must cover these new inputs (attachments included).
- **Docs to update as milestones land:** [ADR-0005](0005-structured-agent-characterization.md) extended; [ADR-0008](0008-execution-sessions-and-atomic-checkout.md) reframed as "the `code` execution kind"; UX.md gains the conversation surface.
- **Design and teachability are first-class deliverables:** the chat surface is designed with the installed design skills (impeccable / emil-design-eng / taste) plus a per-surface `DESIGN.md`; every milestone ships a short "where this lives / how to change it" map so the owner can modify the system himself.
- **Risk:** general-purpose is a larger, less-charted product than code orchestration. We de-risk by shipping the thinnest end-to-end slice first and letting a real project (the home cinema) drive the requirements.
