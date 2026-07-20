-- Budgets & governance (M6). Table shapes follow Paperclip (budget_incidents,
-- approvals, agent_config_revisions). The budget "amount" is the agent's
-- monthly_budget_cents trait rather than a separate budget_policies row — see
-- docs/PAPERCLIP-ALIGNMENT.md and ADR-0012.

-- A per-session budget reservation held while the session is in flight, so
-- concurrent starts can't collectively overrun the cap between checkout and
-- the cost_event landing.
ALTER TABLE agent_task_sessions ADD COLUMN reserved_cents INTEGER NOT NULL DEFAULT 0;

-- Governance flag: starting this agent's tasks requires a human approval.
ALTER TABLE agents ADD COLUMN requires_approval INTEGER NOT NULL DEFAULT 0;

CREATE TABLE budget_incidents (
    id              TEXT PRIMARY KEY,
    company_id      TEXT NOT NULL REFERENCES companies (id),
    agent_id        TEXT NOT NULL REFERENCES agents (id),
    window_start    TEXT NOT NULL,
    threshold_type  TEXT NOT NULL,           -- 'hard' | 'warn'
    amount_limit    INTEGER NOT NULL,
    amount_observed INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'open',
    created_at      TEXT NOT NULL
);

CREATE TABLE approvals (
    id            TEXT PRIMARY KEY,
    company_id    TEXT NOT NULL REFERENCES companies (id),
    type          TEXT NOT NULL,             -- e.g. 'task_start'
    status        TEXT NOT NULL DEFAULT 'pending', -- pending | approved | rejected
    payload       TEXT NOT NULL,             -- JSON: what to do on approval
    summary       TEXT NOT NULL DEFAULT '',
    decision_note TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT
);

-- Forward-only history of an agent's configuration. A rollback appends a new
-- revision (source='rollback') rather than deleting; the chain is never edited.
CREATE TABLE agent_config_revisions (
    id            TEXT PRIMARY KEY,
    company_id    TEXT NOT NULL REFERENCES companies (id),
    agent_id      TEXT NOT NULL REFERENCES agents (id),
    source        TEXT NOT NULL DEFAULT 'patch', -- hire | patch | rollback
    changed_keys  TEXT NOT NULL DEFAULT '[]',
    before_config TEXT NOT NULL,             -- JSON snapshot
    after_config  TEXT NOT NULL,             -- JSON snapshot
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_incidents_company ON budget_incidents (company_id, status);
CREATE INDEX idx_approvals_company ON approvals (company_id, status);
CREATE INDEX idx_revisions_agent ON agent_config_revisions (agent_id, created_at);
