-- Which run holds a task (plan 2026-11-01, the task knows its run). A run
-- moves its task, and is resumed after a restart, only while it is the one
-- recorded here. The checkout sets it; any move by hand, and the run's own
-- applied end, clear it. Status and timestamps cannot say it: a task blocked
-- and put back in progress by hand is `in_progress` again with no new session,
-- and two checkouts can share an instant.
ALTER TABLE tasks ADD COLUMN run_session_id TEXT;

-- Whether the run still held its task when it ended. Its status says how the
-- run went, this says whether that outcome counted: a run of a task the person
-- took back is not reported as done, not counted toward the attempts cap, and
-- not handed on to the next attempt. 1 until a run ends without holding it.
ALTER TABLE agent_task_sessions ADD COLUMN applied INTEGER NOT NULL DEFAULT 1;

-- Runs in flight across the upgrade keep their task: an `in_progress` task is
-- held by its newest live session.
UPDATE tasks SET run_session_id = (
    SELECT s.id FROM agent_task_sessions s
     WHERE s.task_id = tasks.id AND s.status IN ('queued', 'running')
     ORDER BY s.created_at DESC, s.id DESC
     LIMIT 1
)
WHERE status = 'in_progress';
