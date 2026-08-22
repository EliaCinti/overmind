-- M25 (ADR-0033): the right people in, and each in their own companies.

-- Single-use invite codes, stored hashed like session tokens: a leaked
-- database mints no entries.
CREATE TABLE invites (
    id TEXT PRIMARY KEY,           -- sha256 of the code the owner hands out
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_by TEXT REFERENCES users(id)  -- NULL until spent; spent atomically
);

-- Who is inside which company. Presence is the only per-company role for
-- now (ADR-0033 decision 3).
CREATE TABLE company_members (
    company_id TEXT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL,
    PRIMARY KEY (company_id, user_id)
);

-- Companies that predate membership belong to everyone already here: they
-- were created when the instance was one person's, and locking that person
-- out of their own history would be the migration deciding policy.
INSERT INTO company_members (company_id, user_id, added_at)
SELECT c.id, u.id, c.created_at FROM companies c CROSS JOIN users u;
