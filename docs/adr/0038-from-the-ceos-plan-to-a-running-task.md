# ADR-0038: From the CEO's plan to a running task

- **Date:** 2026-08-23
- **Status:** accepted
- **Builds on:** [ADR-0005](0005-structured-agent-characterization.md) (autonomy is a structured, server-enforced trait), [ADR-0012](0012-budgets-and-governance.md) (the approval gate), [ADR-0020](0020-inter-agent-meetings.md) (the human is reached where they are: an approval nobody sees is an agent stuck forever), [ADR-0036](0036-tools-in-the-agents-hand.md) (tools held per agent).

## Context

The owner's first real brief to his CEO — a sketch of the house, the team
approved, Blender in one agent's hand — ended with a task in `todo` and
nothing moving. He wrote down what he expected, and it is the right
expectation:

> I write to the CEO → it understands → it plans → if there are tasks it opens
> them and assigns them properly → it shows me everything → I approve → the
> tasks start.

Three things stood between that sentence and the product, all ours:

1. **The CEO did not know what its teammates held.** The team block named
   names and titles. So the CEO, asked for a Blender model, reasoned from
   first principles that "no external process can attach to a running GUI",
   declared the work impossible, planned it as *code*, and wrote a Python
   script for the human to paste into Blender — while the modeler next to it
   held `blender`, whose whole point is attaching to the open scene.
2. **A `code` task needs a repository.** The company had none. The task the
   CEO planned could never have started — not by a wakeup, not by a human —
   and nothing said so. The work was a knowledge task anyway: a tool is not a
   repository.
3. **Nothing offered the start.** Since M14 the scheduler auto-starts only for
   agents that *act within budget*, and only when woken. An agent that *acts
   with approval* — the hire dialog's own words: *acts on tasks once you
   approve each start* — was never asked for approval, because nobody ever
   attempted the start it would have gated. The task waited for a human to
   find a button the human did not know existed.

## Decisions

1. **The CEO is told what each teammate holds.** The team block becomes
   `- Tobia (Designer Blender) — holds the tools: blender — Blender, via
   BlenderMCP: …`. Same source as the agent's own tools line (ADR-0036): the
   trait and the operator's description.

2. **The prompt says whether `code` can exist here.** With a repository
   connected: *knowledge for research, documents and anything done through a
   tool an agent holds; code only for changes to this company's repository.*
   Without one: *this company has NO repository; every task is knowledge;
   never plan code.* And the server does not rely on the prompt being obeyed:
   **a `code` task planned for a company without a repository is opened as
   `knowledge`**, and the audit event carries `planned_kind: "code"` so the
   correction is visible rather than silent.

3. **A planned task goes to work the way its agent's autonomy says**, the
   moment it is opened — `runner::offer_start`:
   - *acts within budget* → started now (the governance gate still applies if
     the agent is flagged `requires_approval`);
   - *acts with approval* → the start is filed as a `task_start` approval and
     the human is notified, exactly as the governance gate does; approving it
     starts the task;
   - *proposes only* → nothing: a human starts it, nobody is asked.
   This is the existing approval mechanism, reached from one more place. The
   owner's sentence — *it shows me everything, I approve, the tasks start* —
   is now literally the `act_with_approval` path.

4. **Scope: tasks the CEO (or any agent's chat turn) opens.** A task a person
   creates by hand keeps the hand-started flow: asking them to approve what
   they just created would be noise. The scheduler's wakeup rule is unchanged.

## Consequences

- The first live walk of ADR-0036 can happen: the CEO plans with Blender in
  mind, the task is `knowledge`, and the start appears in the inbox.
- Approvals gain a second origin. The approval row is the same shape; the
  inbox already renders it.
- `tests/from_plan_to_work.rs` holds all three: the team block names the
  tool; `code` without a repository opens as `knowledge`; and one test per
  autonomy level — asked, started, waiting.
- A refused start at approval time (the agent paused meanwhile, a capability
  missing) surfaces as the decision's error, as it does for the governance
  gate today.

## Addendum (same day): the chat says when an agent is answering, and one turn at a time

Two more things the owner measured in the same hour. Send a message, switch
to the board, come back: the typing dots were gone and it looked as if nothing
was happening — "answering" lived only in the chat component's memory. And a
second message sent before the reply started a second, concurrent turn.

- `AppState` keeps the set of conversations with a turn in flight (in memory:
  a turn does not survive a restart, so neither should the claim that one is
  running). `GET …/conversation` carries `answering`; the chat asks on every
  load, so the dots survive a page switch and vanish on the same live signal
  the reply arrives on.
- One turn per conversation at a time. A message posted while a turn is in
  flight is stored and the turn is *owed*: when the current one ends, another
  runs only if the thread still ends with the user's words — so a message the
  first turn already read is not answered twice, and one it did not read is
  never left unanswered. Never two turns at once. `tests/answering.rs`.
- The inbox shows what waits on you alone; everything decided or informative
  stays as the record, one toggle away (shown outright when nothing waits).

## Addendum (same evening): the chat's files ride into the task

The first run that reached Blender built a coherent house — laid out wrong,
because the modeler never saw the sketch. The CEO's task said *read
`BozzaCasa.jpeg` and `lettura-schizzo-misure.md` first*; both lived on the
chat thread, and a run copies only the **task's** attachments, so the run
directory was empty and the agent reconstructed the plan from textual hints
(and said so, to its credit, in its summary).

Tasks gain the thread they were born in (`tasks.conversation_id`, set by the
CEO's plan, NULL for hand-made tasks). A task's inputs — what the run copies
in, and what `GET /tasks/{id}/attachments` lists, kept deliberately the same
set — are its own attachments **plus the posted files of its birth thread**
(staged uploads nobody sent do not count). Coarse on purpose: the thread is
the project's folder in the owner's hands, and the cost of an extra file in
the run directory is nothing next to a modeler drawing a house from prose.

## Addendum (same night): an error Overmind can repair arrives with the repair

The multimodal gate did its job — it refused to hand a sketch to an agent
never characterized to look at one — and the owner's reaction named the
missing half: *"si presenta il problema? bene: appare qualcosa che propone di
sistemare, e se l'utente approva, Overmind agisce."* The same principle as the
sign-in flow (M23) and the payer switch (ADR-0037), now generalized:

- **`RunnerError::Remediable { message, remedy }`** → 409 with a
  machine-readable `remedy` beside the message. Never a string-match on the
  English sentence. First remedy: `grant_multimodal`.
- **`POST /agents/{id}/traits`** — the hire's own validated `TraitsPatch`
  applied after hire, recorded as a `patch` revision, exclusivity enforced.
  This closes the "characterization is barely editable after hire" debt from
  NEXT.md, and is how any traits-shaped remedy is applied.
- The task detail shows the refusal with one button — *Characterize {name}
  for visual work and start* — that applies the patch and retries the start.
  `tests/remedies.rs`.

