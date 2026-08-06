-- M13 / ADR-0020: inter-agent meetings. Agents collaborating on their own work
-- can hit a call none of them should make alone. One of them asks for a
-- meeting, naming the room and the reason; the human approves; then, and only
-- then, they deliberate — bounded by a turn cap — until a decision is recorded.

CREATE TABLE meetings (
    id                TEXT PRIMARY KEY,
    company_id        TEXT NOT NULL REFERENCES companies (id),
    topic             TEXT NOT NULL,
    -- Why the convener says the room is needed. This is what the human reads.
    reason            TEXT NOT NULL DEFAULT '',
    -- The agent who called it. NULL when the human convened it directly.
    convener_agent_id TEXT REFERENCES agents (id),
    turn_cap          INTEGER NOT NULL,
    -- requested (waiting on the human) | open (deliberating) | decided
    -- | declined (human said no) | failed (could not run)
    status            TEXT NOT NULL DEFAULT 'requested',
    decision          TEXT,
    -- The approval that gates it (NULL when convened directly by the human).
    approval_id       TEXT REFERENCES approvals (id),
    created_at        TEXT NOT NULL,
    decided_at        TEXT
);
CREATE INDEX idx_meetings_company ON meetings (company_id, created_at);

-- Who is in the room, and in which order they speak.
CREATE TABLE meeting_participants (
    meeting_id TEXT NOT NULL REFERENCES meetings (id),
    agent_id   TEXT NOT NULL REFERENCES agents (id),
    position   INTEGER NOT NULL,
    PRIMARY KEY (meeting_id, agent_id)
);

-- The transcript: one row per contribution, in order.
CREATE TABLE meeting_turns (
    id         TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL REFERENCES meetings (id),
    agent_id   TEXT NOT NULL REFERENCES agents (id),
    ordinal    INTEGER NOT NULL,
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_meeting_turns_meeting ON meeting_turns (meeting_id, ordinal);
