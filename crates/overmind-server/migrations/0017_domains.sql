-- M14 slice 3 (ADR-0021): characterization on two axes.
--
-- The archetype's meaning narrows to the *function* an agent performs; the
-- domain it performs it in becomes a second, orthogonal axis. A domain never
-- widens what the server enforces: it contributes focus areas, declared
-- capabilities and one line of prompt context, on top of the function's
-- defaults.

CREATE TABLE domains (
    id           TEXT PRIMARY KEY,
    slug         TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL,
    -- JSON: DomainPatch — additive only (focus areas, declared capabilities,
    -- a prompt line, and whether the field is visual by nature).
    traits_patch TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

-- Nullable on purpose: every agent hired before this migration keeps working,
-- read as the `general` domain. No backfill, no behaviour change.
ALTER TABLE agents ADD COLUMN domain_id TEXT REFERENCES domains (id);

-- The CEO proposes a function and a field for each hire (ADR-0021). Nullable
-- for the same reason: proposals drawn up before this migration stay readable.
ALTER TABLE org_proposal_members ADD COLUMN domain TEXT;

CREATE INDEX idx_agents_domain ON agents (domain_id);
