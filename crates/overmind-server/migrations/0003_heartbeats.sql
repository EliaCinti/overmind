-- Heartbeat scheduler (M3). agent_wakeup_requests follows Paperclip's shape
-- (source/reason/status/claimed_at/finished_at); full cron-style routines are
-- deferred (see docs/PAPERCLIP-ALIGNMENT.md).

CREATE TABLE agent_wakeup_requests (
    id           TEXT PRIMARY KEY,
    agent_id     TEXT NOT NULL REFERENCES agents (id),
    source       TEXT NOT NULL DEFAULT 'manual',
    reason       TEXT,
    status       TEXT NOT NULL DEFAULT 'queued', -- queued | done
    outcome      TEXT,
    requested_at TEXT NOT NULL,
    claimed_at   TEXT,
    finished_at  TEXT
);

-- Session resume support: the adapter's own session id (e.g. Claude Code's)
-- and how many times the session was resumed after an interruption.
ALTER TABLE agent_task_sessions ADD COLUMN adapter_session_id TEXT;
ALTER TABLE agent_task_sessions ADD COLUMN resumed_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_wakeups_status ON agent_wakeup_requests (status, requested_at);
