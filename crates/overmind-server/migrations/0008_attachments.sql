-- M12 / ADR-0018: attachments on a conversation message.
-- The user can attach files/images to a message; they are copied into the
-- agent's working directory so it can read (or see) them. Bytes live on disk
-- under <data-dir>/attachments/<conversation>/; this table is the index.

CREATE TABLE attachments (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations (id),
    message_id      TEXT REFERENCES messages (id), -- null until linked to a posted message
    filename        TEXT NOT NULL,
    mime            TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    path            TEXT NOT NULL, -- absolute path on disk
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_attachments_message ON attachments (message_id);
CREATE INDEX idx_attachments_conversation ON attachments (conversation_id);
