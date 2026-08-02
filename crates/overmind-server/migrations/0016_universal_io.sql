-- M17: everything in, everything out.
--
-- Two changes, both about the same thing: an agent should be able to take
-- anything you hand it and hand anything back.
--
-- 1. An attachment used to belong to a conversation. It now belongs to a
--    conversation *or* a task — the same bytes, the same upload path, reaching
--    an agent either by chat or by the task it picks up. `conversation_id`
--    becomes nullable; exactly one owner is set, and the CHECK says so.
--
--    SQLite cannot drop a NOT NULL, so the table is rebuilt. Every existing
--    attachment is a conversation attachment, which is what the copy does.
--
-- 2. An artifact gains `size_bytes` and `relative_path`. The path is what
--    makes a subdirectory survive: `research/sources.csv` is a different
--    deliverable from `sources.csv`, and flattening them loses the shape the
--    agent chose. `size_bytes` is what lets the UI decide whether to render
--    something or offer to download it, without reading the file first.

CREATE TABLE attachments_new (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT REFERENCES conversations (id),
    task_id         TEXT REFERENCES tasks (id),
    message_id      TEXT REFERENCES messages (id), -- null until linked to a posted message
    -- Who produced it: the human, or an agent handing something back (M17 D).
    origin          TEXT NOT NULL DEFAULT 'user',  -- user | agent
    filename        TEXT NOT NULL,
    mime            TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    path            TEXT NOT NULL,                 -- absolute path on disk
    created_at      TEXT NOT NULL,
    CHECK ((conversation_id IS NOT NULL) <> (task_id IS NOT NULL))
);

INSERT INTO attachments_new
    (id, conversation_id, task_id, message_id, origin, filename, mime, size_bytes, path, created_at)
SELECT id, conversation_id, NULL, message_id, 'user', filename, mime, size_bytes, path, created_at
FROM attachments;

DROP TABLE attachments;
ALTER TABLE attachments_new RENAME TO attachments;

CREATE INDEX idx_attachments_message ON attachments (message_id);
CREATE INDEX idx_attachments_conversation ON attachments (conversation_id);
CREATE INDEX idx_attachments_task ON attachments (task_id);

ALTER TABLE task_artifacts ADD COLUMN size_bytes INTEGER NOT NULL DEFAULT 0;
-- Path relative to the run's deliverable root, e.g. 'research/sources.csv'.
-- NULL for artifacts written before M17 and for the synthetic "Run output".
ALTER TABLE task_artifacts ADD COLUMN relative_path TEXT;
