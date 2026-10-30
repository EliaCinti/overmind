# A code run works in a clone of its own

*Plan, 29 Oct 2026. Implements [ADR-0052](../adr/0052-a-run-owns-its-repository.md). It supersedes [git runs in the cage](2026-10-29-git-runs-in-the-cage.md) and keeps that plan's caged git for every command after the agent. Both earlier plans of the day stay as the record of how we got here.*

## Why a third plan

The `/code-review` of the caged-git branch (`65efac9`) found that caging the diff leaves the root open. The cage grants the agent all of `repos_dir`, so it can plant hooks and config in the operator's repository. Those then run as root at the next run's `git worktree add`, and as the owner on the host. On a Mac without `repos_dir`, the caged diff cannot even read the repository. The owner chose to remove the grant (ADR-0052, option b).

## The approach

New branch `fix/the-run-owns-a-clone` from `main`, which now has #113. The branch `fix/git-in-the-agents-worktree` is not pushed and does not land. Its pinned `GIT_DIR` and migration 0034 mean nothing when the git dir is inside the run dir. What carries over by hand: `git_in_run` (the caged spawn, `exec git "$@"`), its tests, and the markers under `CARGO_TARGET_TMPDIR`.

1. **`prepare_worktree` → `prepare_clone`:** `git clone --no-hardlinks --no-checkout`, then `checkout -b <branch> <default_ref>`, `remote remove origin`, `base_sha`, `exclude_from_git` on the clone's own `info/exclude`. `WORKTREE_LOCKS` goes.
2. **`sandbox::writable_paths` drops `repos_dir`.** A `Cage` gains read-only paths, used only for diffs of sessions from before, on that repository's git common dir.
3. **One caged git helper** (`git_in_run`) for every command after the agent: `session_diff`, `handoff_text`'s `git status`, `git bundle create`. It gets a timeout and `kill_on_drop`. It does not get the adapter's credentials: `sandbox::command` gains a variant without `credentials()`. It drops the `-c` overrides that only made sense on the operator's repository and keeps `core.fsmonitor=false` and `core.hooksPath=/dev/null`, because they cost nothing.
4. **The way back:** at the end of a `code` run, the caged bundle goes to the run's temp file. Then, as the server in the operator's repository, `git -c transfer.fsckObjects=true fetch <bundle> <branch>:refs/heads/<branch>`, refusing any ref outside `overmind/`. An empty branch (no commits) skips it. A failure is an audit event and a line on the session, never a failed run.
5. **No cage:** see the open question.

## Tests first

- `a_hook_planted_in_the_operators_repository_is_impossible`: the agent tries to write `<repo>/.git/hooks/post-checkout` and `<repo>/.git/config`, and both are denied. Then a second run on the same repository starts with no marker written. On macOS through the profile; skipped where no cage is real.
- `the_runs_commits_come_back_as_its_branch`: the agent commits, and the operator's repository has `overmind/task-…` at that commit after the run.
- `a_bundle_cannot_move_a_ref_outside_overmind`: a clone with a branch named to escape the namespace changes no other ref.
- The four tests of the earlier branch, moved onto the clone: planted `.git` file, textconv, clean filter in the clone's config, and a session from before still diffs. All with a marker outside the cage.
- `the_handoff_status_runs_in_the_cage`: a timed-out run whose clone plants an fsmonitor writes no marker.
- The existing tests that look for the branch or the worktree in the operator's repository are updated, not deleted.

## Open question for the owner

**When no cage is real** (`OVERMIND_SANDBOX=off`, Linux outside the image without Landlock), what does the server's git do over a run's files? Proposal: record on the session whether its run was caged (`caged`, one migration).
- If the run was **not** caged, the agent already ran with the server's reach. The diff runs with the same reach, which is no wider than the agent.
- If the run **was** caged and there is no cage now, the diff is refused with a sentence.

This keeps the diff on hosts without a cage and keeps the guarantee exact.

## Done when

- The tests pass on macOS. The CI runs them on Ubuntu as well, where they skip honestly or pass.
- THREAT-MODEL: the git row is rewritten and a row for "the operator's repository is not in the cage" is added, each naming its tests. CHANGELOG under Fixed, with the cost (disk per run) stated.
- NEXT.md: the cleanup of run dirs is written as a debt with its size.
- The checks in CLAUDE.md, `/code-review medium` and `/security-review` pass. One PR.
