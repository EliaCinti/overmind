# The accounts panel — step 4 of "More than one mind"

- **Date:** 2026-11-12
- **Parent plan:** [2026-10-28-more-than-one-mind.md](2026-10-28-more-than-one-mind.md), step 4 (approved 2026-10-28)
- **Decided in:** [ADR-0051](../adr/0051-an-account-is-connected-not-configured.md) decisions 4, 5, 7; [ADR-0053](../adr/0053-the-cage-grants-nothing-the-host-runs.md) decision 6
- **Branch / worktree:** `feat/the-accounts-panel` at `/Volumes/ExtremeSSD/overmind-wt-accounts`, from `main` `a80ec84`
- **Status:** proposed

## Why a plan of its own

The parent plan gives step 4 one paragraph. Since then three things were added to it, and they change the size of the step:

1. ADR-0053 decision 6 made the agent's own home on the native instance part of step 4: `~/.claude` and `~/.claude.json` stay writable in the cage until then, and a hook planted there runs as the owner.
2. The owner chose the shape of the panel on 11 Nov 2026: a dialog opened from the top bar and from every remedy, not a page.
3. Step 3 landed (`a80ec84`): the API this panel calls exists, and its shapes are known.

## What is true today, read in the code on 12 Nov 2026

- `accounts::for_spawn` (`accounts.rs:629`) returns an empty `Spawn` when the provider has no active account; the CLI then finds the host's login. `Spawn.account_id` is documented as `None` "until the accounts panel exists".
- `cage_home` (`sandbox.rs:433`) falls back to the server's `HOME` when `agent_home` is unset, which is always on the native instance (`OVERMIND_AGENT_HOME` is set only by the `Dockerfile`). `adapter_paths` (`sandbox.rs:520`) then grants the owner's `~/.claude` and `~/.claude.json` writable, and `adapter_read_paths` the owner's `~/Library/Keychains` readable.
- The startup line (`sandbox.rs:225`) says the cage is not a boundary against a hostile agent on a host, naming `~/.claude`.
- The only account-shaped remedy is `connect_plan`, raised by `/economy/pay-with` (`api.rs:586`) and handled in `OrgChart.tsx:352`. Remedies are also rendered in `Inbox.tsx`, `TaskDetail.tsx`, `PayerNotice.tsx`, `ConnectPlan.tsx`.
- The Claude adapter passes `--mcp-config … --strict-mcp-config` (`provider/claude_code.rs:415`): memory does not come from `~/.claude.json`, so a private home does not lose it.
- A run is resumed by session id (`runner::resume_session`, `runner.rs:1585`); Claude Code keeps sessions under the home it ran in.
- The web app has no test runner: its checks are `npm run lint` and `npm run build`.

## The approach — one PR, one commit per sub-step

### 4.1 Measure before changing the home (no code in the repo)

On this Mac, by hand, with a scratch directory as `HOME` and the CLI given `CLAUDE_CODE_OAUTH_TOKEN` (or a key) the way `Spawn` gives it:

- does `claude -p` run headless in an empty home (first-run prompts, `.claude.json` created on its own)?
- does it still try the login Keychain when a token is in the environment, and does it fail if the cage denies it?
- what happens to `--resume <id>` of a session that lives in the old home: an error the runner already treats as "not resumable", or something else?

Each answer goes into the desk's log and, if it costs time to find, into a memory. **If any answer contradicts the slices below, stop and go back to `[PM]`.**

### 4.2 Tests first, watched failing

- `with_no_active_account_a_start_is_refused_with_connect_account` — moved from step 3 in the parent plan.
- `with_no_active_account_a_hire_is_refused_with_connect_account` — ADR-0051 decision 4 says "at hire and at start".
- `on_a_native_instance_the_agent_has_a_home_of_its_own` — with no `OVERMIND_AGENT_HOME`, the home the cage grants is under `data_dir`, never the server's `HOME`.
- `the_cage_grants_nothing_under_the_owners_home` — the writable and readable sets carry no path under the server's `HOME` (`.claude`, `.claude.json`, `Library/Keychains`). Rewrite the existing tests that assert the host grants (around `sandbox.rs:1132`, `:1235`, `:1431`) rather than deleting them silently: each one says in its doc comment what changed.
- whatever 4.1 showed about resume, held by a test.

### 4.3 No implicit host login (backend)

- `for_spawn` fails with a distinct error when the provider has no active account; the runner turns it into `RunnerError::Remediable` with `{"kind": "connect_account", "provider": "<vendor>"}`; hire does the same.
- `Spawn::none` stays for sign-in and for Overmind's own tools; it is the only way to build a `Spawn` without an account.
- `economy`'s "not signed in" state points at `connect_account`. `/economy/pay-with` keeps answering for one release (parent plan), its refusal now carrying `connect_account`.

### 4.4 The agent's own home on the native instance

- When `OVERMIND_AGENT_HOME` is unset, `agent_home` defaults to `<data_dir>/agent-home`, created at boot `0700`, owned like the rest of `data_dir`.
- `cage_home` no longer falls back to the server's `HOME`.
- The startup line stops naming the `~/.claude` hole; `THREAT-MODEL.md` says the hole is closed and names the tests of 4.2.
- The image is unchanged: it already sets `OVERMIND_AGENT_HOME=/home/agent`.

### 4.5 The API client (`web/src/lib/api.ts`)

Types and calls for the routes step 3 shipped: `GET/POST /api/accounts`, `POST /api/accounts/{id}/activate`, `DELETE /api/accounts/{id}`, `POST/GET /api/accounts/sign-in`, `POST /api/accounts/sign-in/code`. Shapes as served by `api.rs`, read from the code, not from the desk.

### 4.6 The panel (UI)

- **`AccountsDialog`**, owner-only, opened from a button in `TopBar` and from every `connect_account` remedy (opening on that provider). Same family as `BackupDialog` and `ConnectionsDialog`.
- One section per provider — Anthropic, OpenAI, Google — listing its accounts: label, kind, source (stored / from the environment), last four of a key, an *active* mark.
- **Connect:** the shape the server declares — paste-back (today's `ConnectPlan` flow, generalised onto `/api/accounts/sign-in*`), key (a field), or *unavailable* with its reason in words.
- **Make active** — one click.
- **Disconnect** — a confirmation. Disconnecting the last active account of a provider says that its agents will be refused until another is connected (confirmed by the owner on 11 Nov). For a subscription it says, from `still_valid_at_provider`, that the token is revoked only at the provider's site. An account from the environment has no disconnect.
- `SignInNotice`, `PayerNotice` and the org view's `EconomyNote` point at the dialog instead of the old blind switch. `connect_plan` is still handled for one release, by opening the dialog on Anthropic.
- Every string through the dictionaries in `web/src/lib/i18n.ts`, in every language they carry.

### 4.7 Docs

`THREAT-MODEL.md`, `ARCHITECTURE.md`, `CHANGELOG.md` (`[Unreleased]`), `NEXT.md`, `ROADMAP.md` (M34), `UX.md` if the dialog sets a pattern, `README.md` where it mentions the host's login. **Every claim is read against the code it describes, and every test it cites is opened and checked to assert that claim** — `threat_model.rs` only checks that the name exists.

### 4.8 Checks, then reviews by Claude Code

The CLAUDE.md checks, green. Then the branch goes to Claude Code for `/code-review medium` and `/security-review` (the diff touches the cage and credentials). Push, PR and merge come after the reviews.

### 4.9 Acceptance, by the owner on the native instance

Casa updated to the merged build. From that moment Casa's agents are refused until an account is connected — the owner connects the right Claude subscription from the dialog straight away and makes it active. Claude Code verifies that an agent's spawn reports that account while the host's `claude auth status` still reports the owner's own, and that nothing under the owner's `~/.claude` changed during a run.

## Done when

- With no active account, hire and start are refused with `connect_account`, and the remedy opens the dialog on that provider.
- From the dialog the owner connects, activates and disconnects Claude accounts (subscription and key); OpenAI and Google are listed with the reason they cannot be connected yet.
- The native instance's agents run in `<data_dir>/agent-home`; the cage grants nothing under the owner's home; the startup line and `THREAT-MODEL.md` say so, held by tests.
- Acceptance 4.9 passed; the CLAUDE.md checks and both reviews ran, every finding fixed or written down.

## Rejected

- **A page instead of a dialog** — the parent plan said "a page"; the owner chose the dialog on 11 Nov 2026, reachable from everywhere a remedy appears.
- **Keeping the host-login fallback behind a flag** — ADR-0051 decision 4 removes it; a flag would keep the bug one setting away.
- **Copying the owner's `~/.claude` into the agent's home** — it would carry the owner's settings, hooks and login into the cage, which is what ADR-0053 closes.

## Out of this plan

- Steps 5–7 of the parent plan (`models`, Codex, Google).
- **Agents that work on the owner's computer** — browsing directories, reading and writing files, using apps (owner's request, 11 Nov 2026). It runs against the direction of ADR-0053 and needs its own `[PM]` and ADR after this plan; nothing of it starts here.
