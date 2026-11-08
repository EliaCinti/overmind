# ADR-0051: An account is connected, not configured

- **Date:** 2026-10-28
- **Status:** accepted (approved by the owner, 2026-10-28)
- **Builds on:** [ADR-0048](0048-a-provider-is-a-capability-not-a-name.md) (a provider is a trait; `sign_in` is one of its members), [ADR-0037](0037-who-pays-is-asked.md) (who pays is asked, and Overmind does the switching), [ADR-0030](0030-how-you-pay-is-a-first-class-fact.md) (how you pay is a first-class fact), [ADR-0044](0044-the-archive-is-the-instance.md) (the archive is the instance)

## Context

The owner, 28 Oct 2026: *"I want an option to sign in with any account I have, across Anthropic, OpenAI and Google."* The request came from a concrete failure: the native instance runs on the wrong Claude account and there is no way to change it from the interface.

What the code does today, read on `main`:

- **One credential, one provider, one place.** A subscription is a single file, `<data_dir>/claude-oauth-token`, created by the sign-in flow (`claude_auth.rs`) and injected as `CLAUDE_CODE_OAUTH_TOKEN` (`sandbox.rs:442`). An API key is never held by Overmind at all: it comes from the server's environment (`ANTHROPIC_API_KEY`, compose passthrough).
- **When neither exists, the host's own login is used.** On the native instance the file is absent, so every agent runs on whatever `claude` on the host is signed into — which is also the owner's personal Claude Code. Two unrelated things share one account by accident.
- **Who pays is one switch.** ADR-0037's `pay-with-plan` decides, for Claude only, whether the environment key is kept away from the agent.
- **The interface can connect, never replace.** `ConnectPlan` renders only on the `connect_plan` remedy, i.e. when nobody is signed in.
- **Google's subscription is not reachable headless.** Since 18 Jun 2026 Gemini CLI no longer serves personal accounts or Google AI Pro / Ultra; Antigravity CLI, their successor, has no headless sign-in (memory #305). A Google **key** (AI Studio) works headless.

## Decision

**1. An account is a thing Overmind holds.** An account is `(provider, kind, credential, label)`, where `kind` is `subscription` or `key`. It lives under `<data_dir>/credentials/<provider>/<account-id>/`, 0700 directory, 0600 files, readable only by the server (ADR-0029). The `label` is what the provider's own probe says the account is (an e-mail, an org name) — never what the owner typed — and is `unknown` when the probe does not say.

**2. Many linked, one active per provider.** An owner may link any number of accounts per provider; exactly one per provider is **active**. The active account is the payer for that provider — this replaces ADR-0037's single `pay-with-plan` switch, which becomes the special case *Anthropic, two accounts, one plan and one key*. Switching takes effect at the next spawn; a run in flight finishes on the account it started with, and each cost event records the account it was billed to, so the ledger says truthfully who paid even across a switch.

**3. The environment is an account too, read-only.** A key in the server's environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`) appears in the panel as an account labelled *from the environment*: selectable, not removable from the interface. When any other account of that provider is active, the variable is removed from the agent's environment — generalising `keep_key_away` — because a key left in place silently outranks a login (measured for Claude Code in memory #207, documented for Codex's auto-created key in ADR-0048).

**4. The host's login is never used implicitly.** When a provider has no active account, its agents are refused at hire and at start with the `connect_account` remedy — not run on whatever the host is signed into. The native instance's current behaviour is the bug this decision removes. Existing instances migrate: `claude-oauth-token` becomes the first linked Anthropic subscription account; an instance with neither file nor key gets the remedy, never a silent fallback.

**5. How an account is connected is the provider's to say.** `sign_in` (ADR-0048) returns one of three shapes, and the panel renders whichever comes back:

- **paste-back** — a URL to open and a code to paste (Claude Code's `setup-token`, as today);
- **device code** — a URL and a code to type there; the server polls (Codex's `--device-auth`);
- **key** — a field to paste a key into.

A provider may declare that a kind is **unavailable**, with the reason in words. Google's subscription is declared unavailable until a measured run shows otherwise; its key is available.

**6. Secrets are known by value, not by prefix.** Every stored credential is registered with the log scrubber by its **exact value**, so redaction no longer depends on a vendor prefix (`sk-ant`) that another vendor does not share. The API never returns a credential: the panel sees kind, label, provider, active flag, and the last four characters of a key.

**7. Connecting, switching and disconnecting are owner-only and audited.** `require_owner` as today. Each is an audit event (`account.connected`, `account.activated`, `account.disconnected`) carrying provider, kind and label — never the credential. Disconnect deletes the files and says, in the interface, what it cannot do: a subscription token is revoked only at the provider's site.

**8. The archive carries accounts only under a passphrase.** ADR-0044's rule for the token — a stored credential makes the export passphrase mandatory — extends to every account. An import restores accounts with their active flags.

## Alternatives considered

**Several accounts active at once, chosen per agent or per company.** The owner considered and declined it (28 Oct 2026): it turns the account into a budget dimension, and every view that says who pays would have to say it per agent. Decision 2 keeps the door open — accounts are already distinct rows with ids, and cost events already carry one — without paying for it now.

**Keys only from the environment, as today.** Rejected by the owner: Google's only headless path today is a key, so "sign in with any account" would exclude Google in practice; and a key that must be edited into a compose file is exactly the shell step the M23 sign-in removed for subscriptions.

**The operating system's keychain.** The container has none, and the native instance would then behave differently from the image on the one thing most worth testing. Files under `/data`, with the cage around them, are what already protects the token today.

**Keep falling back to the host's login.** It is what put the owner's agents on the owner's personal account without anyone choosing it. A fallback nobody can see in the interface is a payer nobody chose.

## Consequences

- The native instance stops running on the host's login. Before the first deploy of this change, the owner connects the right Claude account from the panel, or Casa's agents are refused with a remedy — said in the release notes, not discovered.
- The image ships the Codex and Gemini CLIs (ADR-0048 decision 5 is unchanged: providers are still configured per instance; this ADR is about accounts within them).
- `pay-with-plan` and `/api/economy/pay-with` are superseded by `/api/accounts/{id}/activate`; the old route answers for one release and then goes.
- Google appears in the panel from the first slice that ships a Google provider, with its subscription shown as unavailable and why.
