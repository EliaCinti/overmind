# More than one mind — an accounts panel for Anthropic, OpenAI and Google

- **Date:** 2026-10-28
- **Decided in:** [ADR-0051](../adr/0051-an-account-is-connected-not-configured.md) (accepted with this plan), [ADR-0048](../adr/0048-a-provider-is-a-capability-not-a-name.md); a new ADR for Gemini's road before any Gemini code (step 7)
- **Status:** approved by the owner, 2026-10-28

## The request

The owner (28 Oct 2026): *an option to sign in with any account I have, across Anthropic, OpenAI and Google.* It started from the native instance running on the wrong Claude account with no way to change it from the interface.

Choices the owner made while planning:

- **one active account per provider**, any number linked, switched with one click;
- **subscriptions and keys both** in the panel;
- **no manual stop-gap**: the right Claude account is connected from the panel when it exists, not by hand before;
- finish **M36a slice 2** up to its PR first, then pause M36a.

## What is true today, measured on 28 Oct 2026

- **The native instance (`~/overmind-casa`, :7071) runs on the host's login.** No `claude-oauth-token` in its data dir, so `inject_oauth_token` (`sandbox.rs:442`) injects nothing and agents run on the host's `claude` — the owner's personal Claude Code account, plan `pro`.
- **The interface can connect, never replace.** `ConnectPlan` only renders on the `connect_plan` remedy.
- **Keys are not Overmind's.** They come from the server environment; who pays is ADR-0037's single `pay-with-plan` switch, Claude only.
- **M34 is at slice 3 of 5.** `sign_in` and `models` not started; the six `NEXT.md` findings that travel with `sign_in` are open.
- **Backups:** a stored token makes the export passphrase mandatory (`backup.rs:174`).
- **Codex CLI** allows the ChatGPT plan: `codex login`, `codex login --device-auth` (beta), and `codex exec` reuses the saved login. Not installed here.
- **Gemini — ADR-0048's table is now wrong** (memory #305). Since 18 Jun 2026 Gemini CLI does not serve personal accounts or Google AI Pro / Ultra; Antigravity CLI has no headless sign-in for AI Pro. Gemini CLI with an AI Studio key works headless.

## The approach — one PR per step, each from `main`

Every code step: tests first and watched failing, CLAUDE.md checks (fmt included), `/code-review medium`, `/security-review` (every step from 3 to 7 touches credentials, `/data`, audit or the image).

### 1. M36a slice 2 to its PR

As on desk `m36a-i-freni`. Then M36a pauses; slices 3–4 stay planned in `2026-10-26-m36a-the-brakes.md`.

### 2. Docs PR: ADR-0051 and this plan

ADR-0051 goes to `accepted` with the owner's approval of this plan; `NEXT.md`'s provider table corrected on Gemini; ROADMAP M34 re-scoped around the panel.

### 3. The account store, with Claude Code as its only provider (backend)

ADR-0051 decisions 1–4, 6, 8 — no new provider, no UI.

- `credentials/<provider>/<account-id>/`, the `accounts` table (provider, kind, label, active), the one-active-per-provider invariant in the database, not in code.
- `sign_in` moved into the trait (the 990-line pty machine becomes Claude Code's implementation); the six `NEXT.md` findings closed as part of the move.
- Environment key as a read-only account; `keep_key_away` generalised.
- The host-login fallback **stays** in this step: removing it before the panel exists would leave Casa with no way to connect anything.
- Migration: `claude-oauth-token` → first Anthropic subscription account.
- Redaction by exact value; `account.*` audit events; cost events carry `account_id`; archive rule extended.
- **Tests first**, among them: `a_second_account_of_the_same_provider_is_linked_inactive`, `activating_one_deactivates_the_other`, `an_env_key_is_kept_away_while_a_subscription_account_is_active`, `a_stored_credential_is_redacted_by_value`, `the_old_token_file_migrates_to_an_account`, `a_run_in_flight_is_billed_to_the_account_it_started_on`.

### 4. The accounts panel (UI) — the right Claude account is connected here

A page listing Anthropic, OpenAI and Google, each with its linked accounts: **connect** (subscription or key, in whatever shape `sign_in` returns), **make active**, **disconnect**. The org view's payer switch points here; `/economy/pay-with` answers for one release. Providers not yet supported are listed and say so. Strings through the dictionaries.

**No implicit host login** (ADR-0051 decision 4) lands here, with the panel that repairs it: no active account → refused with `connect_account`, and the remedy opens the panel. Test first: `with_no_active_account_a_start_is_refused_with_connect_account` (moved from step 3).

**Acceptance, by the owner on the native instance:** connect the right Claude subscription, make it active; I verify an agent's spawn reports that account while the host's `claude auth status` still reports the owner's own.

### 5. M34 slice 5 — `models`

The catalogue knows which kind of account reaches which model, so hiring offers only what the active account can pay for.

### 6. OpenAI — measure, then the Codex provider

- **Measure first**, on the owner's ChatGPT plan, recorded into a memory: `codex login --device-auth` on a personal plan, `codex login status` (machine-readable? does it show the auto-created key?), where Codex keeps its login (`CODEX_HOME` per account?), `codex exec --json` (events, session id, failure words, cost or only tokens → `Bound::Money` or `Bound::Turns`).
- Then `provider/codex.rs` and the conformance suite; its subscription connects by **device code**, its key by **key**. If more than that file has to change, the finding is written down as a fault in the trait.

### 7. Google — spike, ADR, then the provider the ADR picks

- **Spike:** Gemini CLI + AI Studio key (JSON output, cost or tokens); Antigravity CLI + AI Pro on the native instance (non-interactive mode? can a caged agent reach the keychain token? expiry).
- **An ADR (the next free number; 0052 went to the run's own clone):** the road. Expected: key now, subscription declared *unavailable* with its reason until `agy` allows headless use.
- **Then the provider it picks**, in the panel. If the ADR picks a shape this plan did not foresee, back to `[PM]`.

## Done when

- From the panel, the owner connects the right **Claude** subscription, a **ChatGPT** subscription and a **Google** key; one is active per provider.
- An agent on each provider completes a real task, the org view says truthfully who pays for each, and switching or disconnecting an account takes effect at the next spawn.
- The native instance no longer uses the host's login; the owner's own Claude Code is unchanged.
- ADR-0051 and the Google ADR accepted; `NEXT.md` and memory #221 no longer claim Gemini CLI takes a personal Google login.

## Rejected

- **Several active accounts per provider** — declined by the owner (ADR-0051, alternatives).
- **A manual token for Casa first** — declined by the owner; Casa stays on the host's login until step 4.
- **Switching the host's `claude` login** — would move the owner's own Claude Code too.
- **Gemini via the old personal login** — closed by Google on 18 Jun 2026.

## Out of this plan, flagged

The Docker instance `~/overmind` (:7070) has failed its health check for ~10 hours with `exec /bin/sh: input/output error` — the VM disk on the external SSD. The owner's call, separately.

## Sources (read 28 Oct 2026)

- https://github.com/google-gemini/gemini-cli/issues/28229
- https://geminicli.com/docs/resources/quota-and-pricing/
- https://github.com/google-antigravity/antigravity-cli/issues/1194
- https://learn.chatgpt.com/docs/auth
- https://github.com/openai/codex/issues/9253
