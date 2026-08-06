-- M11 / ADR-0017: deliverable-agnostic execution.
--
-- A task's `execution_kind` decides what a run produces:
--   'code'      -> today's git worktree + diff (ADR-0008), unchanged.
--   'knowledge' -> no git; the agent works in a scratch dir and produces
--                  artifacts (documents, tables, research, decisions).
-- Default 'code' keeps every existing task and test behaving as before.
ALTER TABLE tasks ADD COLUMN execution_kind TEXT NOT NULL DEFAULT 'code';

-- The general deliverable for a knowledge run. A session registers one or
-- more artifacts; text/markdown lives inline in `content`, large/binary
-- payloads go to `file_path`. Replaces the diff for knowledge tasks.
CREATE TABLE task_artifacts (
    id         TEXT PRIMARY KEY,
    task_id    TEXT NOT NULL REFERENCES tasks (id),
    session_id TEXT NOT NULL REFERENCES agent_task_sessions (id),
    kind       TEXT NOT NULL DEFAULT 'document', -- document | table | research | decision | link
    title      TEXT NOT NULL,
    mime       TEXT NOT NULL DEFAULT 'text/markdown',
    content    TEXT,                              -- inline text/markdown
    file_path  TEXT,                              -- for binary/large payloads
    created_at TEXT NOT NULL
);

CREATE INDEX idx_task_artifacts_task ON task_artifacts (task_id);
CREATE INDEX idx_task_artifacts_session ON task_artifacts (session_id);
