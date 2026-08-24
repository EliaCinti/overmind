-- A conversation's handoff summaries (ADR-0040). When a thread outgrows the
-- turn, the agent writes a summary of the older part; the latest summary plus
-- the messages after `covers_until` are what later turns read. Append-only:
-- a re-compaction writes a new row covering more, it never rewrites one.
CREATE TABLE conversation_summaries (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations (id),
    content         TEXT NOT NULL,
    -- The `created_at` of the last message this summary covers.
    covers_until    TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_summaries_convo ON conversation_summaries (conversation_id, created_at);
