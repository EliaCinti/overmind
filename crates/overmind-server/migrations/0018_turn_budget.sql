-- M18 (ADR-0022): conversational spend enters the ledger, and a room that runs
-- out of money waits instead of dying.
--
-- Cost events need no new table: `cost_events.task_id` and `.session_id` have
-- been nullable since M2, so a chat or meeting turn already fits the ledger.
-- Reservations do, because `agent_task_sessions.task_id` is NOT NULL and
-- letting a chat turn impersonate a task session would corrupt the one thing
-- that table means.

CREATE TABLE agent_turn_reservations (
    id             TEXT PRIMARY KEY,
    company_id     TEXT NOT NULL REFERENCES companies (id),
    agent_id       TEXT NOT NULL REFERENCES agents (id),
    -- What is being paid for: 'chat' | 'meeting'. Kept for the ledger's own
    -- sake — a bill you cannot break down is one you cannot argue with.
    kind           TEXT NOT NULL,
    reserved_cents INTEGER NOT NULL,
    created_at     TEXT NOT NULL,
    -- NULL while the turn is in flight; set when it ends, however it ends. A
    -- reservation that is never released is a budget leak only a restart clears.
    released_at    TEXT
);

-- The hot query is "what does this agent have in flight right now".
CREATE INDEX idx_turn_reservations_open
    ON agent_turn_reservations (agent_id, released_at);

-- `meetings.status` gains `paused`: the room ran out of budget mid-deliberation
-- and is waiting for you to top it up or let the window roll over. Not a
-- terminal state — the transcript is intact in `meeting_turns` and deliberation
-- resumes from the ordinal it stopped at. The column is free text with the set
-- documented in 0010; nothing to alter for the status itself.
ALTER TABLE meetings ADD COLUMN paused_agent_id TEXT REFERENCES agents (id);
ALTER TABLE meetings ADD COLUMN paused_note TEXT;
