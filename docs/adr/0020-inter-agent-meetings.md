# ADR-0020 — Inter-agent meetings

- Status: accepted
- Date: 2026-07-31
- Builds on: [ADR-0019](0019-conversational-agents-and-cross-impact.md) (conversational agents), [ADR-0013](0013-memory-over-mcp.md) (memory), [ADR-0012](0012-budgets-and-governance.md) (approvals), [ADR-0006](0006-audit-log-and-task-lifecycle.md) (audit).

## Context

Agents already collaborate: a specialist opens a task for a teammate, escalates
to the leader, works its own tasks (ADR-0019). Collaboration that real means
sometimes hitting a call **none of them should make alone** — one that changes
what colleagues are building, or sits above the role of whoever found it.

Today the only ways out are bad ones: the agent guesses, or it stalls. What is
missing is the thing a company does here — get the right people in a room,
settle it, and carry on.

Two dangers to avoid. A free-form group chat never ends and costs unbounded
tokens. And agents that can summon each other at will spend the human's money
on conversations the human never asked for.

## Decision

**A meeting is an automation the agents start, the human allows, and that
always ends in a recorded decision.**

**1. An agent asks.** While working — in a conversational turn, or mid-task —
an agent can request a meeting: it names the `topic`, the `participants` it
wants in the room, a `turn_cap`, and, crucially, the **`reason`**, in its own
words. The convener is always in its own room; named teammates are resolved
against the roster; if nothing resolves, the org leader is brought in, because
an agent asking for a meeting is at minimum asking for its boss.

Two channels, because agents work in two places:
- **Conversational turn**: a `meeting` object in the plan JSON it already returns.
- **At work on a task**: a `MEETING_REQUEST.json` file in its working directory.
  A file, not stdout — the last JSON line of a run belongs to the adapter's own
  result envelope, so stdout is not the agent's to speak on.

**2. The human is notified, and decides.** The request creates a `meetings` row
in status `requested`, an **approval** (`type = 'meeting_request'`, ADR-0012)
and a **notification** (see below) — in one transaction. **No agent turn runs
and no token is spent** until the approval is granted. Approving moves the
meeting to `open` and starts the deliberation; rejecting moves it to `declined`
and tells the agent that asked, with the human's note.

**3. They deliberate, round-robin and bounded.** Participants speak in turn, up
to `turn_cap` (clamped to `[1, 12]`). Each turn runs the agent (adapter) with
the topic + the transcript so far + its role, and returns a validated JSON
contribution:

```json
{ "say": "<one or two sentences>", "decision": "<optional: the group's decision>" }
```

The turn cap is the budget for this slice: at most `turn_cap` (+ one closing
turn) adapter runs, so a meeting can never spin forever. (Per-token budget
reservation, as at task checkout, is deferred — like conversational turns today.)

**A meeting that only agrees is worse than no meeting.** Round-robin agents
drift into "+1" and into restating the last turn, and a room that never tests
its options reaches a decision nobody stress-checked. The prompt carries that
load, and it is part of the decision, not a detail:
- every turn is given the **convener's reason**, not just the topic — the room
  must know what it is actually there to settle;
- the **first speaker frames the choice**: the two or three real options and the
  trade-off between them, from what its role is responsible for. Restating the
  topic is explicitly out;
- **later speakers must add, not nod**: agreement has to name the condition,
  cost or risk it carries; disagreement has to give the reason and the
  alternative; restating a made point is forbidden, and an agent with nothing to
  add says so and defers;
- a `decision` is only for genuine convergence, and must be **concrete enough to
  act on tomorrow** — the closing turn states both the call and why it beats the
  alternative that was raised.

**4. It concludes with a decision, always.** If any turn returns a `decision`,
the meeting ends with it early. Otherwise, on reaching the cap, the **chair**
(the leader among the participants, else the first) takes one closing turn where
a decision is mandatory. The meeting moves to `decided`.

**5. The decision goes back to work.** A meeting is only worth holding if it
changes what happens next, so on conclusion:
- it is **audited** (`meeting.decided`) and **stored to organizational memory**
  via `store_decision` (best-effort, ADR-0013);
- it is **injected into the prompt** of every participant's next task run and
  next conversational turn, as settled context they must act on and not
  re-litigate;
- every participant is **woken** (`agent_wakeup_requests`, source `meeting`) so
  it picks its work back up. The wakeup is a request, not a command: the
  scheduler still enforces autonomy and budget (ADR-0005/0012), so an agent
  that needs a human to start work still needs one.

**6. The human is told the outcome**, through the same mechanism that asked.

## The notification mechanism

Meetings need the company to be able to **reach the human** — so this ADR also
introduces notifications as a first-class thing, not a meeting detail. A
notification is a **durable row** (`notifications`) *and* a **live push** over
`/ws` (`{"type": "notification", …}`). The row is the record — nothing is lost
while the app is closed; the push is only the fast path. It is written in the
same transaction as the event that caused it, so the human is never told about
something that did not happen, nor misses something that did.

A notification carries who is telling you (`agent_id`), what to open
(`subject_type`/`subject_id`), and — when it is actionable — the `approval_id`
to decide on. `meeting.requested` is the first actionable kind; escalations and
task approvals are the obvious next users.

## Consequences

- New modules `meeting.rs` and `notify.rs`; API: `POST/GET /companies/{id}/meetings`,
  `GET /meetings/{id}`, `GET /companies/{id}/notifications`,
  `POST /notifications/{id}/read`, `POST /companies/{id}/notifications/read`.
  Approving/rejecting reuses `POST /approvals/{id}/decision` (ADR-0012).
- The human can still convene a meeting directly (`POST /companies/{id}/meetings`);
  asking for it *is* the approval, so it opens immediately. Agents cannot reach
  that path — they always go through the request.
- The meeting runs in the background; the transcript and decision land as they happen.
- A meeting that cannot run (adapter missing, timeout) is closed as `failed`
  with a `meeting.failed` audit event and notification — never left open forever.
- **UI:** the bell becomes one **inbox** for everything an agent wants you to
  know or decide, answered inline (`Inbox.tsx`); live notifications also arrive
  as toasts (`Toaster.tsx`); a **Meetings** surface lists the rooms and shows the
  transcript arriving turn by turn, with the decision pinned under it
  (`Meetings.tsx`). Gated task starts (ADR-0012) now raise a notification too,
  so the inbox is the single place the company reaches you — two bells would
  have been two places to miss something.

## Rejected alternatives

- **Free-form group chat**: rejected — unbounded and never concludes.
- **Agents convening each other without the human**: rejected — it spends money
  on conversations the human never sanctioned. The approval gate is the point.
- **Requiring a second agent to "second" the request before notifying the
  human**: rejected — it burns an adapter turn per invitee *before* the human
  has even said yes, to answer a question the human is about to answer anyway.
- **A single "synthesizer" agent deciding alone**: rejected — that is just an
  agent turn (ADR-0019), not a deliberation.
- **No hard cap, stop on consensus only**: rejected — consensus may never come;
  the cap guarantees termination and bounds cost.
- **Notifying only inside the convener's chat thread**: rejected — a request
  that needs an answer cannot live where you may never look.

## Where this lives

- `crates/overmind-server/migrations/0010_meetings.sql` — meetings schema.
- `crates/overmind-server/migrations/0011_notifications.sql` — notifications schema.
- `crates/overmind-server/src/meeting.rs` — request, approve/decline, bounded run, decide, wake.
- `crates/overmind-server/src/notify.rs` — record + live push.
- `crates/overmind-server/src/ceo.rs` — the `meeting` field in a turn's plan; shared `run_adapter`.
- `crates/overmind-server/src/runner.rs` — `MEETING_REQUEST.json` pickup; decisions in the task prompt.
- `crates/overmind-server/src/api.rs` — meeting + notification routes, the approval branch.
- `crates/overmind-server/tests/meetings.rs` — the acceptance tests.
- `web/src/components/Inbox.tsx` — the bell: one feed, decisions inline.
- `web/src/components/Meetings.tsx` — the rooms, the live transcript, the decision.
- `web/src/components/Toaster.tsx` — live notifications as they land.
- `web/src/lib/live.ts` — the `notification` frame on the existing socket.
