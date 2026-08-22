# Security

Overmind runs agents that execute a real CLI on the machine that hosts it. That makes "what does it defend against" the most important question about it, and the answer is written down rather than implied: **[docs/THREAT-MODEL.md](docs/THREAT-MODEL.md)** states every boundary Overmind claims, the mechanism that holds it, and the test that holds the claim — and, just as plainly, what it does *not* defend against (anyone with the machine, a malicious adapter or memory binary you configured, the network). Please read it before reporting; a gap that is already named there is a known limit, not a vulnerability.

## Reporting a vulnerability

If you believe you have found a way to cross a boundary the threat model **claims** — an agent reaching outside its cage, a non-member reading a company, a session forged or a chain edited without detection, a way to run the CLI without a credential — please report it privately:

- Use GitHub's **private vulnerability reporting** on this repository ("Report a vulnerability" under the Security tab), or
- write to the maintainer listed on the repository profile.

Include what you did, what you expected the boundary to hold, and what you observed — a command or a request that reproduces it is worth more than a description. You will get an acknowledgement within a few days; fixes land as a tagged release with the threat model updated to name the new held-by test.

## What is in scope

- The cage (`sandbox-exec` on macOS; uid split and Landlock in the container): an agent reading or writing what the threat model says it cannot.
- The door: authentication, sessions, invites, membership on every company surface.
- The audit chain: an edit that verification fails to detect.
- The MCP surface for outside callers and for agents: a token reaching another company, a tool doing more than its contract.

## What is out of scope

- Anything reachable only by someone who already has the host: a shell, root, the Docker socket, the data directory. *The door guards the port; nothing guards a hostile host.*
- A malicious `OVERMIND_AGENT_CMD` or `OVERMIND_MEMORY_CMD`: they are commands you configured, and neither binary is verified.
- An **unclaimed** instance: it is open by construction until the owner claims it. Claim early.
- The model provider's own behaviour, rate limits or billing.

## Supported versions

Pre-1.0: only the latest release receives fixes.
