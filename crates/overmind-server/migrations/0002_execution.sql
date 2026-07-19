-- Execution layer (M2). Names follow Paperclip's schema
-- (project_workspaces, agent_task_sessions, cost_events) — see
-- docs/PAPERCLIP-ALIGNMENT.md.

CREATE TABLE project_workspaces (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects (id),
    name        TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'local_path',
    cwd         TEXT NOT NULL,           -- path of the git repository agents work on
    default_ref TEXT,                    -- branch/ref worktrees start from (default: repo HEAD)
    is_primary  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE TABLE agent_task_sessions (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL REFERENCES tasks (id),
    agent_id       TEXT NOT NULL REFERENCES agents (id),
    adapter_type   TEXT NOT NULL DEFAULT 'claude_code',
    status         TEXT NOT NULL DEFAULT 'queued', -- queued | running | completed | failed
    branch         TEXT NOT NULL,
    workspace_path TEXT NOT NULL,        -- the session's isolated git worktree
    base_sha       TEXT,                 -- repo commit the worktree started from (diff base)
    output         TEXT,                 -- captured stdout+stderr, persisted
    exit_code      INTEGER,
    last_error     TEXT,
    created_at     TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT
);

CREATE TABLE cost_events (
    id                  TEXT PRIMARY KEY,
    company_id          TEXT NOT NULL REFERENCES companies (id),
    agent_id            TEXT NOT NULL REFERENCES agents (id),
    task_id             TEXT REFERENCES tasks (id),
    session_id          TEXT REFERENCES agent_task_sessions (id),
    provider            TEXT NOT NULL,
    model               TEXT NOT NULL,
    input_tokens        INTEGER NOT NULL DEFAULT 0,
    cached_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens       INTEGER NOT NULL DEFAULT 0,
    cost_cents          INTEGER NOT NULL,
    occurred_at         TEXT NOT NULL,
    created_at          TEXT NOT NULL
);

CREATE INDEX idx_sessions_task ON agent_task_sessions (task_id);
CREATE INDEX idx_cost_events_company ON cost_events (company_id);
