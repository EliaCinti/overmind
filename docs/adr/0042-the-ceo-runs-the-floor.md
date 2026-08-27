# ADR-0042: The CEO runs the floor

- **Date:** 2026-08-27
- **Status:** accepted
- **Builds on:** [ADR-0038](0038-from-the-ceos-plan-to-a-running-task.md) (`offer_start` and its autonomy gates; chat files ride into tasks), [ADR-0041](0041-the-ceo-writes-back-on-its-own.md) (digests, and their "no work unasked" rule), [ADR-0012](0012-budgets-and-governance.md) (the approval gate).

## Context

The owner read a relaunch plan his CEO had written — a *good* plan: which
task gates which, what to run in parallel, what not to spend on — and then
noticed who its executor was: **him**. "Rilancia adesso — cinque, in
parallelo… appena Ludovica risponde…" The CEO had the judgement of a floor
manager and the hands of a consultant. His words: *"il CEO comanda e
controlla — deve proporre i rilanci, e i risultati devono passare agli
agenti successivi; sempre previa mia autorizzazione."*

The gap was structural: a chat turn's plan had one verb — open new tasks.
No way to start or relaunch existing work, no way to say "when X delivers,
Y begins with X's results in hand".

## Decisions

1. **`start` — the CEO starts and relaunches existing work.** The plan may
   carry `"start": ["<title of an open task>"]`. Each resolves to the newest
   open task with that title (`backlog`/`todo`/`blocked`; a blocked task
   returns to the queue first — a relaunch is exactly that) and goes through
   **`offer_start`'s autonomy gates unchanged**: within-budget runs, with-
   approval lands in the inbox, propose-only waits. Unknown titles and
   unassigned tasks are skipped, never an error — a stale title must not eat
   the turn.

2. **`after` — dependencies as data.** A planned task may carry `"after":
   "<title>"` (a task from the same plan, or open on the board). It is
   stored as `tasks.depends_on` (migration 0031), the task is **not offered
   at creation**, and when the dependency's session completes:
   - the dependency's **artifacts are copied into the dependent's
     attachments** — the same table `place_inputs` already reads, so the
     dependent opens with the deliverables in its working directory;
   - `depends_on` is cleared (a second completion must not re-trigger);
   - the dependent is offered by its agent's autonomy.
   An unknown `after` title is dropped: a dependency on nothing must not
   freeze a task forever.

3. **A digest proposes, never spends.** The digest prompt may return
   `"start": [...]` — each files a `task_start` **approval** directly,
   whatever the agent's autonomy. ADR-0041's rule survives refined: an
   unprompted turn still cannot open tasks, and now it can *propose* the
   next starts — which land in the inbox for one click. The loop the owner
   asked for: deliver → the CEO writes "done X, propongo Y e Z" → approve →
   the floor moves.

4. **Recursion is broken by construction.** Releasing a dependent starts a
   session whose completion may release further dependents; the release runs
   in its own spawned task, and the `offer_start` edge is type-erased
   (`boxed_offer_start`) — chains of any length, no cyclic future.

## Consequences

- The CEO's plans become executable: "rilancia questi cinque" is one plan,
  approvals where the gates say so, instead of five manual button-presses.
- Pipelines exist: gate → dependents, with deliverables handed over.
  `tests/the_floor.rs` holds all three verbs.
- The human's control points are unchanged in kind: autonomy per agent,
  `requires_approval`, budgets, and every digest-proposed start is an
  approval by definition.
- Not done, deliberately: cross-thread orchestration (an agent's turn may
  only start work in its own company), fan-in ("after A *and* B") — one
  dependency per task until a real plan needs more.
