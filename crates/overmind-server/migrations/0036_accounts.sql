-- An account is a thing Overmind holds (ADR-0051).
--
-- Until now there was one credential, in one fixed file, for one provider. A
-- row per account lets an owner link any number per provider, and the partial
-- unique index below is the rule that exactly one of them pays: it lives here,
-- in the database, so no code path can forget it.
--
-- `provider` is the vendor whose bill it is (anthropic, openai, google), not
-- the CLI that spends it -- that is `cost_events.provider` since 0032.
--
-- The secret itself is never in this table. A stored account's secret is a
-- 0600 file under `credentials/<provider>/<id>/`; an environment account's is
-- whatever the variable named in `env_var` holds at boot.
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('subscription', 'key')),
    source TEXT NOT NULL CHECK (source IN ('stored', 'environment')),
    env_var TEXT,
    -- What the provider's own probe says the account is; never typed.
    label TEXT NOT NULL DEFAULT 'unknown',
    -- A key's last four characters, so two keys can be told apart.
    last_four TEXT,
    active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
    created_at TEXT NOT NULL,
    CHECK ((source = 'environment') = (env_var IS NOT NULL))
);

CREATE UNIQUE INDEX accounts_one_active_per_provider
    ON accounts (provider) WHERE active = 1;

CREATE UNIQUE INDEX accounts_one_row_per_variable
    ON accounts (env_var) WHERE env_var IS NOT NULL;

-- Who paid for a turn, recorded at spawn: switching accounts takes effect at
-- the next run, and the ledger has to say so truthfully across the switch.
-- NULL for every row from before accounts, and for a run on the host's login.
ALTER TABLE cost_events ADD COLUMN account_id TEXT;
