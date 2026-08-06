-- ADR-0019: conversations are with an agent (not only the CEO).
-- Rename the column and move from one thread per company to one per (company, agent).
-- The "CEO conversation" is now just the thread with the org leader.

DROP INDEX idx_conversations_company;
ALTER TABLE conversations RENAME COLUMN ceo_agent_id TO agent_id;
CREATE UNIQUE INDEX idx_conversations_company_agent ON conversations (company_id, agent_id);
