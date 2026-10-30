# The server's git on the agent's state runs in the agent's cage

*Plan, 29 Oct 2026. It supersedes [the earlier plan of the same day](2026-10-29-git-in-the-agents-worktree.md), whose central claim the `/code-review` of its branch disproved. That plan is kept as the record of how we got here.*

## Why the first plan was wrong

The first plan made the server's git name the worktree's real git directory, so it never reads the `.git` file the agent controls. The claim it rested on was that this directory "lives in the operator's repository, out of the agent's reach". **That is false whenever `OVERMIND_REPOS_DIR` is set**, which is always the case in the image.

`sandbox::writable_paths` grants the agent write access to `repos_dir` on purpose: a code run commits, and a worktree keeps its metadata under `<repo>/.git/worktrees/<id>`. So the agent can rewrite `.git/config`, `config.worktree`, or the entry's `commondir`. Naming the git directory then closes one door and leaves the config itself open. Listing dangerous keys one by one (`core.fsmonitor`, hooks, ext-diff, textconv) cannot keep up either: `filter.<name>.clean` and `filter.<name>.process` run on `git diff` against the working tree, `core.sshCommand` runs on a lazy fetch, and the list has no end.

## The approach

**Close the class, not the keys.** Every git command the server runs over state the agent can write — today `session_diff`, plus the handoff's `git status` once #113 lands — is spawned **through the cage**, as the agent runs: `sandbox::command` with that run's confinement (macOS: `sandbox-exec` with the run's profile; image: the agent uid through the same path the adapter takes). A planted command then runs with exactly the agent's privileges, which is nothing the agent could not already do. The commands the server runs **before** the agent starts (`worktree add`, `rev-parse`, `exclude_from_git`) stay as they are: at that point the state is still the server's.

What carries over from the first branch, kept as defence in depth:
- the git directory recorded at creation and passed as `GIT_DIR` / `GIT_WORK_TREE`. On a machine without `repos_dir`, the admin directory really is out of reach, and naming it beats asking a file the agent owns;
- `core.fsmonitor=false`, `core.hooksPath=/dev/null`, `--no-ext-diff --no-textconv`.

Corrected from the first branch:
- **`recover_git_dir`** trusted `gitdir` files in an agent-writable directory. It goes. A session from before the migration is diffed through the cage like any other, without a recorded `GIT_DIR`.
- **`session_diff`** no longer needs `company_id`, so the `JOIN tasks` goes too.
- **The CHANGELOG and the threat-model row** say what is true. The diff runs when a person opens it, not at the end of every run. The guarantee is "runs no wider than the agent", not "never runs".

## Tests first

- The two tests already on the branch stay, with their assertion sharpened: the marker file is written **outside** the run dir, so a caged command cannot write it. They hold "the planted command did not run as the server" whether git honoured the key or not.
- `a_clean_filter_in_the_repository_config_does_not_run_as_the_server`: the agent writes `filter.planted.clean` into the repository's own `.git/config` (writable through `repos_dir`) and `* filter=planted` into `.gitattributes`. The test runs where `repos_dir` is set, so the first plan's fix alone fails it.
- `a_session_from_before_the_migration_still_diffs`: a session with no `git_dir` still shows its change.

## Open question for the owner

How the cage is entered for a diff on Linux depends on what `sandbox::confinement` offers for a run directory **after** the run has ended. It may be bubblewrap or Landlock in the image, by ADR-0038. If it offers nothing there, a caged diff is not possible and the honest fallback is to **refuse** the diff with a sentence, never to run it uncaged. That fallback has to be agreed before it ships.

## Done when

- The tests pass.
- The threat-model row is rewritten to the narrower, true guarantee and names its tests.
- The checks in CLAUDE.md, `/code-review medium` and `/security-review` all pass.
- This lands as one PR together with the first commit, and #113's `git status` follows the same path.
