-- Whether a session's run was ever caged (ADR-0052). The server's git over a
-- run's clone runs in the run's cage; where this machine has no cage, it may
-- still run -- with the same reach the agent had -- but only over a run that
-- was itself uncaged. A run that was caged is never read uncaged because the
-- cage was turned off afterwards: its clone was the agent's.
--
-- 1 once any of its runs was caged, 0 if none was. Sessions from before this
-- migration have none, and are treated as caged: nobody can say they were not.
ALTER TABLE agent_task_sessions ADD COLUMN caged INTEGER;

-- The git dir a session's clone was made from: where its branch goes back to
-- when the run ends. Recorded, not resolved again from the task, which may
-- have moved to another project -- or the company gained a second repository
-- -- while the run was under way. Sessions from before have none: they were
-- worktrees, and their commits are in the repository already.
ALTER TABLE agent_task_sessions ADD COLUMN repo_git_dir TEXT;
