# ADR-0028: Overmind as an MCP server for callers outside it

- **Date:** 2026-08-14
- **Status:** accepted

## Context

M9 has been half-built since M8. [ADR-0027](0027-agents-reach-memory-through-overmind.md)
put a real MCP server at `/mcp` so caged agents could reach memory, and every caged run has
gone through it since. What that ADR did not answer is the milestone's actual criterion:

> a Claude Code session **outside** Overmind creates a task via MCP.

The two callers look alike and are not. An agent Overmind spawned is untrusted, ephemeral,
and already inside a cage; the run mints it a token that dies with the session. A Claude
Code session in someone's editor is the *owner* — it has whatever the person has — but it
is durable, it lives in a config file that other tools read, and no session exists to hang
a token on.

Two facts constrain what a token can honestly claim here.

**There is no authentication in Overmind, and this is written down.**
[THREAT-MODEL.md](../THREAT-MODEL.md) says it plainly: *"the boundary is the physical
machine"*. `/api` is open on loopback. Anything an MCP tool can do, `curl` can already do
without a token. So a token on this endpoint is **not** a security boundary, and an ADR
that implied otherwise would be adding the third thing this project has documented, believed
and wired to nothing.

**What a token *is* good for is identity and withdrawal.** ADR-0027's rule — *"a request
names no company; the company is resolved from the token"* — is why no argument exists that
could reach another company's work. And a credential you paste into a config file must be
one you can take back without restarting the server or invalidating the others.

## Decision

**A token says who is calling, and what a caller of that kind may do. Nothing else changes
about the endpoint.**

### Two kinds of caller, one endpoint

| caller | credential | lives | may call |
|---|---|---|---|
| an agent Overmind is running | `agent_task_sessions.mcp_token`, minted per run | until the run ends | `recall`, `why`, `brain_watermark`, `changed_since` |
| an integration outside Overmind | `company_tokens.token`, issued and revoked by the owner | until revoked | `create_task`, `list_tasks`, `get_task`, `verify_audit`, `list_events` |

The grant is a property of the credential, not of the request. `tools/list` answers with the
tools *that caller* may call, so an outside session never sees memory tools and an agent
never sees `create_task`. This matters more for the second direction than the first: agents
already open tasks, through the CEO's plan layer, where a human approves the shape of the
work (M12, M15). A caged agent with a direct `create_task` would route around that gate — not
by breaking it, but by never meeting it.

### An integration files work. It does not authorise any.

`create_task` files into `backlog`, unassigned, exactly as the UI's own dialog does.
Deliberately absent, and not by omission:

- **starting a task** — that spends money and runs an agent. Since M6 the decision to spend
  has been a human's, gated by budget and approval, and an MCP tool that starts work would
  be a second door into the room those gates guard.
- **assigning** — assignment is an act of organization, and the org chart is where it
  belongs (M5).
- **hiring, approving, budgets** — same reason, more so.

Filing is a *request*; starting is *authority*. The split is the whole point of the tool
list, and it is why `create_task` is safe to hand to a durable credential.

### Validation is shared; projection is not

`create_task` goes through the same function the HTTP handler uses — one place that decides
what a valid task is, so priority, `execution_kind` and goal ownership cannot drift between
the two doors. This project has already paid for parallel copies of a rule once
(`agent_command`, ADR-0021).

The read tools do *not* share the UI's response shapes. A model reading a board wants a
compact list it can hold; the SPA wants ids and timestamps for rendering. Those are two
projections of the same rows, not two copies of one rule, and treating them as duplication
would produce a shape that serves neither.

### The token is stored as it is used

`company_tokens.token` is plaintext, like `agent_task_sessions.mcp_token` before it. Hashing
is what you do when the store is a weaker boundary than the credential; here the store is
`overmind.sqlite` on the owner's disk, and anyone who can read it can read everything the
token would reach. Hashing would buy the *appearance* of protection and cost the ability to
show a token again. It is a v4 UUID, not v7: a v7 encodes the time it was minted, and a
secret should not be predictable in any dimension (ADR-0027).

Tokens carry a label, because a credential you cannot tell apart from another is one you
will never revoke. Revocation is a timestamp, not a delete: the audit log names the token
that filed a task, and a row that vanished would leave that name pointing at nothing.

## Alternatives considered

**One server-wide token in the environment (`OVERMIND_MCP_TOKEN`).** Much less code — no
table, no endpoints, no UI. Rejected on two counts: it would have to name a company in every
tool call, undoing ADR-0027's structural guarantee that no such argument exists; and it
cannot be withdrawn from one integration without breaking every other, which is the one
thing a durable credential must be able to do.

**No token at all — `/api` is already open.** Honest about the threat model and wrong about
the mechanism. The endpoint resolves a company *from* the credential; with no credential
there is nothing to resolve, so every tool would take a `company_id` argument. That is the
same regression as above, arrived at from the other side.

**Reuse the run tokens by minting a session for the outside caller.** A fake session with no
task and no agent, existing only to hang a token on, would put rows in
`agent_task_sessions` that describe work nobody did — and the scheduler resumes orphaned
sessions (ADR-0009).

**Let the outside caller read memory too.** Genuinely useful — *"what did we decide about
X"* is a good question to ask from an editor. Held back because every tool on a durable
credential is durable exposure, and the memory view already answers it in the UI. Nothing
here forecloses it: the grant is a table, and adding a tool to the integration list is a
line.

## Consequences

- M9's criterion becomes testable: a caller with a company token creates a task, and it
  appears on the board with the audit event behind it.
- Tokens are a thing to manage. There is now a place in the UI to issue and revoke one, and
  a secret shown exactly once — the first user-facing secret this product has.
- The tool list is grant-dependent, so `tools/list` can no longer be a constant. That is the
  cost of the split, and it is small.
- What an integration can do is now a decision recorded here rather than a consequence of
  what happened to be easy. Anything added to that list is an ADR amendment, not a patch.
