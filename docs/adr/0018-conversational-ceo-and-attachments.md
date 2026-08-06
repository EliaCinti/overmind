# ADR-0018: Conversational CEO and attachments

- **Date:** 2026-07-27
- **Status:** accepted

## Context

[ADR-0016](0016-general-purpose-conversational-company.md) names M12: the **front door** of the general-purpose company — the user talks to a **CEO** in a chat, the CEO **decomposes intent into tasks and dispatches** to the team, and the conversation carries **file/image attachments**. M11 ([ADR-0017](0017-knowledge-execution-and-artifacts.md)) made a dispatched task able to produce a document, so a CEO plan can now actually be executed. M12 builds the conversation, the CEO turn, and attachments on top of it.

## Decision

1. **Conversation + messages.** `conversations` (`id`, `company_id`, `title`, `created_at`) — start with **one CEO thread per company** (extensible to per-project later). `messages` (`id`, `conversation_id`, `role` = `user | ceo | system`, `content`, `created_at`), append-only and audited (`conversation.created`, `message.posted`).

2. **A CEO turn is a bounded run over the conversation.** Posting a user message triggers the **CEO agent** (an agent with an `is_ceo` flag / a `ceo` archetype) to run: input = conversation history + its org chart (the team it can staff work to) + memory context; output = **(a)** a reply `message` and **(b)** a **structured plan** — zero or more task proposals `{ title, description, execution_kind, suggested_role_or_assignee }`. Overmind applies the plan **server-side**: it creates the tasks (audited `task.created`) and, within governance, may dispatch them. **Structured-first** ([ADR-0005](0005-structured-agent-characterization.md)): the CEO's actions are validated JSON the server enforces, never free prose that silently mutates state. Budget and approval gates apply to the CEO turn like any run.

3. **Dispatch.** First slice: the CEO opens tasks in `todo`; auto-start is opt-in and reuses M6 governance (the assignee's autonomy/budget). The board shows what the conversation produced — **chat drives, board is the ledger** ([ADR-0016](0016-general-purpose-conversational-company.md)).

4. **Attachments.** `attachments` (`id`, `company_id`, `conversation_id?`, `task_id?`, `filename`, `mime`, `byte_size`, `path`, `created_at`). A multipart upload endpoint stores the file under `<data-dir>/attachments/<id>/` (a DB row points at it — mirrors the artifact `file_path` pattern, [ADR-0017](0017-knowledge-execution-and-artifacts.md)). Messages and tasks reference attachments. When a task runs, its attachments are copied into the agent's cwd (worktree or scratch dir) and their paths passed via env; agents declared **multimodal** (M14) receive image paths for vision. Size and type limits are enforced.

5. **UI.** A new **Chat** surface alongside Board/Org: a message thread plus a composer with text and file/image upload. The CEO's replies and the tasks it opened appear inline (a message can read "opened 3 tasks" linking to the board). Same [ADR-0010](0010-frontend-stack-and-live-updates.md) stack, designed with the team's design skills.

## Alternatives considered

- **CEO creates tasks by calling Overmind over MCP (M9).** Cleaner long-term, but couples M12 to M9. Returning a structured plan the server applies is self-contained and simpler for the first slice; revisit when M9 lands.
- **Free-form CEO prose that we parse into actions.** Brittle and unauditable. Rejected for structured plan output.
- **Store attachments as blobs in SQLite.** Bloats the DB and complicates streaming. Rejected: files on disk, a row holds the path — same shape as artifacts.
- **One conversation per task.** Too granular for "talk to the CEO". Rejected: one CEO thread per company to start.
- **Replace the board with the chat.** The board is the accountability spine. Rejected: keep both.

## Consequences

- **A third execution path.** Besides `code` and `knowledge` task runs, the **CEO turn** is an *orchestration run*: it reuses the adapter + memory + budget machinery, but its output is a **plan + reply**, not a diff or artifact. It gets its own module.
- **Governance/audit widen:** CEO turns cost budget; new events (`conversation.created`, `message.posted`, `attachment.uploaded`); the M10 prompt-injection review must now cover message content **and uploaded attachments** — an uploaded file is untrusted input.
- **First slice (M12 accept):** the user posts *"find the best 4K editions of the Avengers saga"* in chat; the CEO replies and opens **one** `knowledge` task for a research agent; an uploaded image reaches the agent that runs; the audit chain still verifies.
- **Docs to update as it lands:** [ADR-0005](0005-structured-agent-characterization.md) gains `is_ceo` / a `ceo` archetype; UX.md gains the chat surface; attachments + the multimodal capability are formalized in M14.
- **Teachability:** ships the "where this lives / how to change it" map — the `conversations`/`messages`/`attachments` tables, a `ceo` orchestration module, the upload endpoint, and the Chat UI.
- **Risk:** the CEO's plan quality and prompt-injection surface (chat + files) are the two riskiest parts; both are contained by structured, server-validated actions and by keeping the first slice to a single-task plan.
