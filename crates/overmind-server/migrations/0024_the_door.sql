-- M24 (ADR-0032): the boundary moves off the machine and onto a credential.
--
-- One owner today, users tomorrow (M25): the table is plural because the
-- schema outlives the milestone, and nothing but the claim endpoint enforces
-- "one" -- atomically, at insert time.
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    -- argon2id, PHC string format: parameters and salt travel inside it.
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Server-side sessions. The id is the SHA-256 of the token the browser
-- holds: a leaked database mints nothing.
CREATE TABLE auth_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX idx_auth_sessions_user ON auth_sessions(user_id);
