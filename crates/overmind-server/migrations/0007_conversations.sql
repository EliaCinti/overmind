-- M12 / ADR-0018: the conversational layer. The user talks to a CEO in a
-- thread; the CEO's turn produces a reply and opens tasks for the team.

-- One CEO thread per company to start (extensible to per-project later).
CREATE TABLE conversations (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies (id),
    ceo_agent_id TEXT NOT NULL REFERENCES agents (id),
    title        TEXT NOT NULL DEFAULT 'CEO',
    created_at   TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_conversations_company ON conversations (company_id);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations (id),
    role            TEXT NOT NULL, -- user | ceo | system
    content         TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_messages_conversation ON messages (conversation_id, created_at);
