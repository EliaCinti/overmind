-- M23 (carried): where each person left off. A fresh browser used to land on
-- the first company rather than the last one used, because the memory lived
-- in localStorage. Per user, not per browser -- two people on one instance
-- have two answers. No foreign key on purpose: a deleted company leaves a
-- stale pointer that the client resolves against the list it can see, and a
-- cascade would make deleting a company rewrite user rows.
ALTER TABLE users ADD COLUMN last_company_id TEXT;
