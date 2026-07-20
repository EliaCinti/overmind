# ADR-0009: Heartbeat scheduler, timeouts, restart recovery, autonomy-gated wakeups

- **Date:** 2026-07-20
- **Status:** accepted

## Context

M3 needs many agents working in parallel safely, agents that resume across interruptions, and a periodic "heartbeat" that wakes agents up — Paperclip's `routines` + `agent_wakeup_requests` substrate. Full cron-style routines are deferred; M3 builds the scheduler they will sit on.

## Decisions

1. **In-process registry of live runners.** `AppState.running: Arc<Mutex<HashSet<session_id>>>`. A session marked `queued`/`running` in the DB but absent from this set is an *orphan* — its process died with the server. This is how a single-node server distinguishes "someone is on it" from "needs recovery" without a heartbeat/lease column.
2. **Heartbeat = recover orphans + process wakeups**, on a fixed interval (`OVERMIND_HEARTBEAT_SECS`, default 30s), `MissedTickBehavior::Skip` so a slow beat never stampedes. `beat()` is public so tests drive it deterministically instead of sleeping for real intervals.
3. **Orphan recovery with a grace window.** Only sessions older than `2 × heartbeat` are considered, because `start_task` commits the session row *before* the runner registers; the window avoids racing a just-started session. Recovery re-runs the agent in the existing worktree with a "you are resuming" prompt, bumps `resumed_count`, and (when the adapter supports it) passes the adapter's own session id via `OVERMIND_RESUME_SESSION_ID`.
4. **Timeouts release, failures block.** A session over `session_timeout_secs` (default 3600) is killed (`kill_on_drop`) and the task is **released** to `todo`, unassigned — someone else can retry (`task.released` audit event). An agent that *runs* but exits non-zero **blocks** the task (needs human attention). Distinguishing "infrastructure lost the run" (release) from "the work failed" (block) matters for who acts next.
5. **Wakeups enforce autonomy (ADR-0005).** A queued wakeup only auto-starts the oldest `todo` task if the agent's traits grant `act_within_budget`; `propose_only` / `act_with_approval` agents get a recorded outcome explaining a human must start the work. This is the autonomy trait becoming a server-enforced gate, not prompt text — exactly the UX↔security link from ADR-0005.

## Alternatives considered

- **A heartbeat/lease column per session** (claim with an expiring lease) — robust for multi-node, but this is a single-node self-hosted server; the in-process set plus the age window is simpler and sufficient. The lease approach is the natural M-later upgrade if Overmind ever runs multiple server processes.
- **Auto-releasing blocked tasks too** — rejected: a failed run often means the task itself is wrong; silently re-queuing would loop forever. Human triages `blocked`.
- **Cron routines now** — deferred; wakeup requests are the primitive, routines schedule them later.

## Consequences

- Worktrees still are not cleaned up (carried over from ADR-0008) — resume relies on the worktree persisting, so cleanup must be review-driven (M4) or explicit, never automatic on finish.
- Branch names use the full session id (`overmind/task-<tag>-sess-<uuid>`): UUIDv7 ids minted in the same millisecond share their prefix, so a short prefix collided across parallel sessions (`cannot lock ref`). Uniqueness must ride on the full id.
- Single-writer SQLite serializes all the scheduler's writes against request handlers, so no additional locking is needed for the registry/DB interplay.
