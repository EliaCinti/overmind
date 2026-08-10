-- M8 slice 2 (ADR-0025): what produced a memory is Overmind's fact.
--
-- ADR-0015 decided that the completion-time `store_memory` stays
-- orchestrator-authoritative "with the task as provenance". That has been an
-- intention and not a fact: the brain receives a title and a description and
-- nothing that points back. This table is the fact.
--
-- `memory_ref` is what the *provider* called the thing it just stored, taken
-- from the tool result. TEXT, not INTEGER: the MCP contract (ADR-0003) promises
-- three tool names and free-form results, not a numeric id. Wadachi happens to
-- answer with one; nothing says the next provider will.
--
-- `subject_type` + `subject_id` is the same polymorphic pair `notifications`
-- uses, and for the same reason: a memory comes from a task, a decision comes
-- from a meeting, and inventing two tables for one relationship would be
-- inventing a distinction the UI does not have.
--
-- No foreign key on `subject_id` — it points at two different tables by
-- design, and a link whose task was deleted is still true about the past. The
-- browser tolerates a subject that has gone (ADR-0025).
CREATE TABLE memory_links (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies (id),
    -- 'memory' | 'decision' — which of the two write tools produced it. Kept
    -- because the two are listed separately and a ref is only unique per kind.
    kind         TEXT NOT NULL,
    memory_ref   TEXT NOT NULL,
    -- 'task' | 'meeting'
    subject_type TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    -- Denormalized so the browser can name the subject even when the row it
    -- points at is gone. Cheap, and the alternative is a dangling label.
    subject_title TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

-- The hot query: rendering a page of memories, then attaching each one's
-- subject. Ordered to match how the browser asks — one company, one kind, many
-- refs.
CREATE UNIQUE INDEX idx_memory_links_ref
    ON memory_links (company_id, kind, memory_ref);

-- The reverse: "what did this task teach the organization".
CREATE INDEX idx_memory_links_subject
    ON memory_links (subject_type, subject_id);
