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
