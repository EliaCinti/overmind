-- M24 → M25: roles arrive with signup. The first account ever created owns
-- the instance; everyone after is a member. What a role changes today is
-- deliberately one thing -- billing (the subscription sign-in) is the
-- owner's -- so the column is enforced from birth, never decorative.
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'member';
-- Every pre-existing user came from the owner claim.
UPDATE users SET role = 'owner';
