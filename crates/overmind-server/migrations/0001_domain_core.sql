-- Vocabulary follows Paperclip's canon (see docs/PAPERCLIP-ALIGNMENT.md):
-- companies, tasks (statuses: backlog/todo/in_progress/in_review/blocked/done/cancelled),
-- projects + goals.

CREATE TABLE companies (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE archetypes (
    id             TEXT PRIMARY KEY,
    slug           TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL,
    default_traits TEXT NOT NULL, -- JSON: AgentTraits
    created_at     TEXT NOT NULL
);

CREATE TABLE roles (
    id         TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies (id),
    title      TEXT NOT NULL,
    reports_to TEXT REFERENCES roles (id),
    created_at TEXT NOT NULL
);

CREATE TABLE agents (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies (id),
    role_id      TEXT REFERENCES roles (id),
    archetype_id TEXT NOT NULL REFERENCES archetypes (id),
    name         TEXT NOT NULL,
    traits       TEXT NOT NULL, -- JSON: AgentTraits (archetype defaults + overrides, merged at hire time)
    custom_brief TEXT,          -- additive only, never overrides enforced traits (ADR-0005)
    status       TEXT NOT NULL DEFAULT 'active', -- active | paused | terminated
    created_at   TEXT NOT NULL
);

CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    company_id  TEXT NOT NULL REFERENCES companies (id),
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);

CREATE TABLE goals (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects (id),
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);

CREATE TABLE tasks (
    id                TEXT PRIMARY KEY,
    company_id        TEXT NOT NULL REFERENCES companies (id),
    goal_id           TEXT REFERENCES goals (id),
    title             TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'backlog',
    priority          TEXT NOT NULL DEFAULT 'medium', -- low | medium | high | urgent
    assignee_agent_id TEXT REFERENCES agents (id),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE TABLE audit_events (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    company_id TEXT,
    task_id    TEXT,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL, -- JSON, hashed as stored
    created_at TEXT NOT NULL,
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL
);

-- Append-only enforcement at the storage layer. The hash chain makes
-- tampering *detectable*; these triggers make casual mutation *impossible*
-- through the SQL surface.
CREATE TRIGGER audit_events_no_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only');
END;

CREATE TRIGGER audit_events_no_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only');
END;
