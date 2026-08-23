-- The thread a task was born in (ADR-0038). A task the CEO opens from a chat
-- inherits that chat's files: the run copies them beside the task's own
-- attachments, and the task lists them. NULL for tasks created by hand.
ALTER TABLE tasks ADD COLUMN conversation_id TEXT REFERENCES conversations (id);
