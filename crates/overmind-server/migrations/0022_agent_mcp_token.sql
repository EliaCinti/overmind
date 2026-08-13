-- M9 foundation / M8 slice 3 (ADR-0027): how a run proves who it is.
--
-- Agents reach memory through Overmind rather than through the filesystem,
-- because ADR-0023's cage does not reach the brain directory and widening it
-- would spend a boundary to buy something mediation gives for free.
--
-- The token is the run's identity, not merely a lock on the door. A request to
-- the MCP endpoint carries no company id and no brain path: Overmind resolves
-- both from this row. An agent cannot ask for another company's memories
-- because it cannot name a company at all.
--
-- On the SESSION for the same reason the watermark is (0021): identity belongs
-- to one execution. A retry is a different run and gets a different token, so a
-- token recovered from a stale config file is not a key to the next attempt.
--
-- NULL once the run ends. Invalidation is a write, not an expiry: a token whose
-- run is over must stop working the moment it is over, and a clock is a worse
-- answer than a fact. NULL is also the ordinary state for every historical row.
ALTER TABLE agent_task_sessions ADD COLUMN mcp_token TEXT;

-- The hot path: one lookup per tool call, on a value that must be unique.
-- Partial, so the many NULLs of finished runs do not collide with each other.
CREATE UNIQUE INDEX idx_sessions_mcp_token
    ON agent_task_sessions (mcp_token)
    WHERE mcp_token IS NOT NULL;
