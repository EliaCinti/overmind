-- M13 / ADR-0020: the notification mechanism — how the company reaches the
-- human. Agents work on their own; when something needs attention (an agent
-- asking to convene a meeting, the decision that meeting reached) it lands
-- here. Durable, so nothing is lost while the app is closed; the live push
-- over /ws is the fast path, not the record.

CREATE TABLE notifications (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies (id),
    kind         TEXT NOT NULL,          -- meeting.requested | meeting.decided | ...
    title        TEXT NOT NULL,
    body         TEXT NOT NULL,
    -- Who is telling you: the convener, the agent that escalated. NULL for the
    -- system itself.
    agent_id     TEXT REFERENCES agents (id),
    -- What to open when you act on it (e.g. 'meeting' + the meeting id).
    subject_type TEXT,
    subject_id   TEXT,
    -- Set when the notification is actionable: the approval to decide.
    approval_id  TEXT REFERENCES approvals (id),
    read_at      TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_notifications_company ON notifications (company_id, created_at);
CREATE INDEX idx_notifications_unread ON notifications (company_id, read_at);
