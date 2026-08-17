-- M9 (ADR-0028): how a caller from *outside* Overmind proves who it is.
--
-- 0022 gave a running agent an identity that dies with its run. This is the
-- other caller: a Claude Code session in someone's editor, an automation, a
-- script. It is the owner rather than an agent, so it may file work and read
-- the board — but it is durable, it lives in a config file other tools read,
-- and there is no session to hang a token on.
--
-- It carries the same structural guarantee as 0022 and for the same reason: the
-- company is resolved from this row, so no tool takes a company argument and
-- there is nothing a caller could set to reach another company's work.
--
-- Not a security boundary, and the threat model says so: `/api` is open on
-- loopback and anyone with the machine can already do everything this reaches.
-- What the token buys is identity and *withdrawal* — a credential you handed to
-- one integration and can take back without touching the others.
CREATE TABLE company_tokens (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    -- Why it exists, in the owner's words. A credential you cannot tell apart
    -- from another is one you will never revoke.
    label        TEXT NOT NULL,
    token        TEXT NOT NULL UNIQUE,
    created_at   TEXT NOT NULL,
    -- Answers "is this one still in use?" before answering "should I revoke
    -- it?". Written on use, best-effort: a lost update here costs nothing.
    last_used_at TEXT,
    -- Revocation is a timestamp, not a DELETE. The audit log names the token
    -- that filed a task, and a row that vanished would leave that name pointing
    -- at nothing.
    revoked_at   TEXT
);

-- The hot path: one lookup per tool call.
CREATE INDEX idx_company_tokens_token ON company_tokens (token);
CREATE INDEX idx_company_tokens_company ON company_tokens (company_id);
