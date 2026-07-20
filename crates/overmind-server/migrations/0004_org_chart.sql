-- Company org chart (M5). Aligns with Paperclip: the reporting hierarchy lives
-- on the agents themselves (an agent reports to another agent; the root's
-- manager is the human owner), plus a free-text title. The separate `roles`
-- table from M1 (never given an API, always empty) is dropped in favor of
-- this model. See docs/PAPERCLIP-ALIGNMENT.md and ADR-0011.

ALTER TABLE agents ADD COLUMN title TEXT;
ALTER TABLE agents ADD COLUMN reports_to TEXT REFERENCES agents (id);

ALTER TABLE agents DROP COLUMN role_id;
DROP TABLE roles;

CREATE INDEX idx_agents_reports_to ON agents (company_id, reports_to);
