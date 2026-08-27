-- M30 (ADR-0042): a task may wait for another. When the dependency's run
-- completes, the dependent inherits its deliverables as inputs and is
-- offered to start by its agent's autonomy. NULL = no dependency.
ALTER TABLE tasks ADD COLUMN depends_on TEXT REFERENCES tasks (id);
