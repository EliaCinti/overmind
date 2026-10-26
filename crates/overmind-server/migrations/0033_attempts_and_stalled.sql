-- The attempts cap (ADR-0050 §1, §10): a task that fails the same way three
-- times stops as `stalled` instead of going back to the queue. A time-out
-- released the task to `todo` with nothing counted, and the next wakeup picked
-- it first again — bounded only by the wallet.
--
-- `failure_cause` is the class of a failed session's outcome, as data
-- (`timed_out`, `infra`, `agent_failure`, `empty_handed`), so the count never
-- string-matches `last_error`. Sessions failed before this migration have none
-- and count for nothing: the cap starts from today, not from history.
ALTER TABLE agent_task_sessions ADD COLUMN failure_cause TEXT;

-- When a person last put a stalled task back in the queue: only failures after
-- it count, because the person has looked.
ALTER TABLE tasks ADD COLUMN attempts_since TEXT;
