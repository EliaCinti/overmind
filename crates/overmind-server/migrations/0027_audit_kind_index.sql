-- M25: the actor made legible. The surfaces where decisions show resolve
-- *who* from the audit chain itself (the actor rides inside every hashed
-- payload since M24), so the chain is read by kind for each decided row. An
-- index on kind keeps that a seek, not a scan. An index, never a column: the
-- append-only triggers are untouched and the hash formula does not change.
CREATE INDEX idx_audit_events_kind ON audit_events (kind);
