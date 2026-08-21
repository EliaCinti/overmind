# ADR-0032: Authentication — the boundary moves off the machine

- **Date:** 2026-08-21
- **Status:** accepted (design; implementation is M24)

## Context

Since M0 the security boundary has been the machine: Compose binds loopback,
the browser boundary is CORS + WebSocket origin, and the threat model says
plainly that anyone who can reach the port can spawn processes. That trade
was right for one person on their own laptop, and it stops being right the
moment the owner wants what he asked for today: Overmind on his Mac, a friend
running their own on Windows or Linux, either of them reachable from another
room — and, later, two real people inside one company.

Authentication is the milestone the Known Gaps have pointed at since M10
turned out to be the cage around the *agent* rather than a door in front of
the *caller*. It has to come before any shared or hosted Overmind, and it has
to be built the way the owner asked: properly.

## Decisions

1. **One owner account, claimed on first run.** When no account exists, the
   interface offers exactly one thing: create the owner (a name and a
   password). The claim is atomic — of N concurrent attempts one INSERT wins
   — so a race cannot mint two owners. No default credentials, no password in
   an environment variable: an env var lands in `docker inspect`, shell
   history and compose files, which is where passwords go to be found.

2. **Passwords are argon2id, nothing less.** The RustCrypto `argon2` crate,
   per-password salt, parameters calibrated to ~100ms on a modest machine.
   The hash is what is stored; the password itself must never reach a log,
   an audit event, or an error message.

3. **Sessions are random, server-side, and stored hashed.** 256 bits from the
   OS, handed to the browser as `HttpOnly; SameSite=Strict; Path=/` — and
   `Secure` when `OVERMIND_COOKIE_SECURE=on`, which any TLS deployment must
   set. The server stores only the SHA-256 of the token: a leaked database
   does not mint sessions. Sliding expiry (30 days), revocation by logout,
   and a `sessions` view the owner can clear.

4. **Everything is guarded except the door itself.** Every `/api` route
   requires a session, with three exceptions: the login and first-run-claim
   endpoints; a redacted `/api/health` that answers `{"status":"ok"}` and
   nothing else when unauthenticated (probes and orchestrators keep working,
   the economy and plan windows stop leaking); and the static files of the
   SPA, which are the login screen's own bytes. `/mcp` keeps its own bearer
   tokens — per-run tokens and connection tokens are already an
   authentication, scoped tighter than any cookie.

5. **CSRF: same-site cookies plus a content-type contract.** `SameSite=Strict`
   stops the browser sending the cookie cross-site at all; on top, every
   state-changing route requires `content-type: application/json`, which a
   cross-site form cannot produce. Belt and braces, both cheap.

6. **The WebSocket authenticates like everything else.** The cookie rides the
   upgrade request; the existing origin check stays. An unauthenticated
   socket gets the close code, not a silent stream.

7. **Login attempts are rate-limited and audited.** A small in-memory bucket
   (per source, a handful of attempts per minute) on top of argon2's natural
   cost; every success, failure and logout is an audit event with the actor
   on it. The actor lands on *all* audit events from M24 on — today it is
   always the owner, and it is exactly what M25 needs to mean anything.

8. **Overmind does not terminate TLS.** Doing TLS properly in-process means
   certificates, renewal, and a permanent side-channel of getting it wrong.
   The documented paths to reach Overmind from another machine are, in
   order: **Tailscale/WireGuard** (encryption and identity at the network
   layer, zero certificates, the right default for "my Mac and my friend's
   PC"); or a **reverse proxy** (Caddy) that owns TLS with
   `OVERMIND_COOKIE_SECURE=on` behind it. Compose stays loopback-only by
   default either way: reaching out is a decision, never a surprise.

## Toward several people in one company (M25, designed now, built later)

The friend on Windows or Linux runs **his own** Overmind today — M24 makes
each instance safe to reach. Two people inside **one** company is a different
thing, and honesty about its shape now saves a redesign later:

- **One server, one truth.** A shared company lives on one Overmind that both
  people reach (a VPS, or one person's machine over Tailscale). There is no
  multi-server sync in this design — replicating an audit chain and a brain
  across machines is a different product.
- **Users generalize the owner.** The owner invites by one-time link; a user
  is a row, a password hash, and sessions of their own. Roles start minimal:
  owner and member, the difference being user management and company
  deletion, not day-to-day work.
- **The actor is the point.** Approvals, budget changes, task decisions and
  chat turns carry *which human* — the audit chain finally answers "who
  approved this" with a name, which is the entire reason a shared company
  can be trusted at all. The `actor` field M24 introduces is this, arriving
  early.
- **Isolation stays organizational.** Company scoping already exists
  server-side; M25 makes membership the filter. Per-company secrets stay out
  of scope until someone actually needs them.

## Alternatives considered

- **OAuth / OIDC (log in with GitHub…)** — right for a SaaS, wrong for a
  self-hosted box that must work offline on a LAN. A password the owner
  chose, hashed properly, has fewer moving parts and no third party. OIDC can
  arrive later as an *addition* for hosted deployments.
- **mTLS client certificates** — strongest transport identity, and nobody
  installs client certs on a friend's Windows laptop. Rejected on usability.
- **A bearer token in a config file** — how half of self-hosted tools do it,
  and how their tokens end up in pastebins. Cookies with HttpOnly do not
  transit through people's clipboards.
- **Building TLS in** — see decision 8. Refused as a permanent liability.

## Consequences

- The first-run experience gains one screen (claim the owner), and every
  subsequent visit a login. M22's sign-in notice and onboarding flow sit
  *behind* the login, unchanged.
- The API grows `users`, `sessions`, login/logout/claim endpoints, an auth
  extractor on every route, and `actor` on audit events. The web client
  gains a login screen and a session-expired path.
- CI gains the adversarial tests a door deserves: wrong password, expired
  session, forged cookie, cross-site content-type, unauthenticated WS, and
  the race on first-run claim.
- The threat model's "anyone with the machine" section is rewritten: the
  boundary becomes the credential, and the machine paragraph moves to "what
  we still do not defend against" (a hostile root on the host owns the
  process; that never changes).
