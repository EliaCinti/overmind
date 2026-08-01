-- M15: the CEO proposes an organization, the human decides.
--
-- You tell the CEO your idea in chat; it answers with a *proposed* team — who
-- to hire, in what role, reporting to whom, and why. Nothing is hired on that
-- alone: the proposal is a durable object you approve, trim or refuse, exactly
-- like a meeting request (ADR-0020). The alternative path is untouched: hire
-- everyone yourself and wire the org chart by hand.

CREATE TABLE org_proposals (
    id           TEXT PRIMARY KEY,
    company_id   TEXT NOT NULL REFERENCES companies (id),
    -- The CEO that drew it up.
    proposed_by  TEXT NOT NULL REFERENCES agents (id),
    -- Why this shape, in its own words. This is what the human reads first.
    summary      TEXT NOT NULL DEFAULT '',
    -- proposed (waiting on the human) | accepted | rejected
    status       TEXT NOT NULL DEFAULT 'proposed',
    approval_id  TEXT REFERENCES approvals (id),
    -- The human's reason for refusing, fed back into the CEO's next prompt so
    -- it does not re-propose the same team.
    decline_note TEXT,
    created_at   TEXT NOT NULL,
    decided_at   TEXT
);
CREATE INDEX idx_org_proposals_company ON org_proposals (company_id, created_at);

CREATE TABLE org_proposal_members (
    id             TEXT PRIMARY KEY,
    proposal_id    TEXT NOT NULL REFERENCES org_proposals (id),
    position       INTEGER NOT NULL,
    name           TEXT NOT NULL,
    archetype      TEXT NOT NULL,
    title          TEXT,
    -- The NAME of another member of this proposal, or NULL to report straight
    -- to the CEO. Names, not ids: nobody has been hired yet. Resolved to real
    -- agent ids when the proposal is accepted.
    reports_to     TEXT,
    brief          TEXT,
    -- Why this person is on the team — shown next to them in the UI.
    rationale      TEXT,
    -- The human can drop individual members before accepting the rest.
    excluded       INTEGER NOT NULL DEFAULT 0,
    -- Filled in on acceptance, so the proposal stays an auditable record of
    -- what was actually hired from it.
    hired_agent_id TEXT REFERENCES agents (id)
);
CREATE INDEX idx_org_proposal_members ON org_proposal_members (proposal_id, position);
