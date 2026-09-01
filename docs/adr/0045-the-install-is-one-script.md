# ADR-0045: The install is one script, and the door hands out the first code

- **Date:** 2026-09-01
- **Status:** proposed
- **Builds on:** [ADR-0043](0043-the-compose-file-is-the-installers.md) (the compose file is the installer's, and the asset is where you get it), [ADR-0014](0014-docker-deployment.md) (one deployment image), [ADR-0032](0032-the-door.md) (the first run claims the instance).

## Context

Installing Overmind today is five things a person does by hand: get a container
engine, download the compose file, export a credential, run
`docker compose up -d --pull always`, then open a browser and claim the
instance before anybody else does. ADR-0043 removed the worst of it — no clone,
one file, one command that both starts and updates — and the site grew per-OS
tabs after the first fresh-machine installs. What is left is still a procedure
a person reads and performs, and two of its steps are the ones that bit on the
two-machine walk: the engine was never covered, and *"claim it before anyone
else"* is advice, not a mechanism.

That second one is the sharper edge. `require_owner` returns `Ok` while `users`
is empty, so an unclaimed instance is open by construction (`docs/NEXT.md`, the
debts from the first live walk; M34-B named the same thing from the other
direction). On loopback it is theoretical. The moment somebody binds Overmind
to a tailnet address — which the wiki teaches, because sharing a company is the
product — the first stranger to reach the port owns the company.

The owner's constraint, given plainly: the graphical road **must cost nothing**.
Signing was priced before it was rejected — Apple Developer at $99/year for
notarization, Azure Trusted Signing at $9.99/month for Windows, an EV
certificate at $280+/year — and an unsigned `.pkg` or `.msi` greets a person
with the warning macOS and Windows reserve for malware, which is worse than the
terminal it was meant to replace.

## Decision

1. **One script is the installer**, in the repository at `scripts/install.sh`
   (POSIX `sh`, macOS and Linux) with a Windows twin `scripts/install.ps1`. It
   does seven things and nothing else:

   1. names the OS and architecture, and refuses what it cannot install with a
      sentence rather than an error;
   2. **looks for a container engine and never installs one silently.** Absent,
      it prints the one line for that platform — `curl -fsSL https://get.docker.com | sh`
      on Linux, Docker Desktop or `brew install colima` on macOS, Docker Desktop
      with the WSL2 backend on Windows — and exits non-zero. A prerequisite
      stated is not the same thing as a system changed behind someone's back;
   3. creates its directory (`~/.overmind`, or `$XDG_DATA_HOME/overmind`);
   4. downloads `docker-compose.yml` **from the release asset** and checks it
      against the SHA-256 the release publishes. Not from `main`: what a machine
      executes should be what a release published. Per ADR-0043 the asset is
      where you get the file, not a pin — its `image:` still names `latest`, and
      the one update line keeps working;
   5. runs `docker compose up -d --pull always`;
   6. waits for the server to answer, then prints the URL **and the setup code
      if there is one** (below);
   7. closes by saying the two things that matter afterwards: that the same
      command is also the update, and where the data lives — with the archive as
      the way out (`/wiki/backup`).

   It refuses to run as root, writes nothing outside its directory, and prints
   its own version so a support question has an answer.

2. **`https://overmind.eliacinti.dev/install.sh` redirects to the release
   asset**, and the release workflow attaches both scripts the way it already
   attaches the compose file. The one-liner is
   `curl -fsSL https://overmind.eliacinti.dev/install.sh | sh`
   (`irm … /install.ps1 | iex` on Windows), and the site shows the careful form
   beside it — download, read, check the SHA-256, run. A redirect rather than a
   copy on the web server, so there is no second file to keep in step.

3. **An instance that is not on loopback demands a setup code to be claimed.**
   At first boot, when the bind address is not a loopback address and no owner
   exists, the server mints a single-use code, keeps only its hash, and writes
   it once to the log; the claim form asks for it and the door refuses a claim
   without it. On loopback nothing changes — there is no code to copy, and the
   quickstart stays four lines. This is what makes the install *secure* rather
   than merely convenient, and it closes the debt in `docs/NEXT.md` for the one
   route where an open instance is not a feature. The general
   `ClaimedOwner` extractor for the other owner-only routes stays that debt's
   own work, not this one's.

4. **The graphical road comes second, free, and is not chosen yet.** It wraps
   this same script — never a second implementation of these seven steps. The
   candidates, all costing nothing, are written down so the choice is made on
   evidence rather than taste: a Homebrew cask and a `winget`/Scoop manifest
   (conventional and free, still a typed command); a double-clickable wrapper of
   the same script (free, and on macOS it needs one right-click → Open, because
   a downloaded file is quarantined); a Docker Desktop extension (free, but
   Extensions ship **disabled by default** since August 2026, so nobody would
   find it); and the home-server app stores — Umbrel, CasaOS, Unraid, a
   Portainer template — which are genuinely one click and free, for an audience
   Overmind has not yet met. Decided when we know who actually installs it.

## Consequences

- The install becomes one line to read out loud, and the same line on three
  platforms. The per-OS tabs on the landing stay, because the *prerequisite*
  differs per platform even when the install does not.
- The release workflow gains two assets and a published SHA-256. This depends
  on a release that carries them: today
  `releases/latest/download/docker-compose.yml` answers **404**, because
  `files:` entered `release.yml` after `v0.2.2` was tagged. **0.2.3 is the first
  release the installer can point at**, and the script cannot ship before it.
- A new supply-chain surface: a script executed from a URL. It is bounded the
  way the ecosystem bounds it — a release asset rather than a branch, a
  published checksum, a careful form shown next to the convenient one, and no
  privilege escalation — and it is honest to say `curl | sh` asks for trust that
  reading the file first does not.
- The door gets a second way to refuse a claim, and the threat model gains a
  row with the test that holds it. A person who binds to a tailnet address and
  loses the log line has to reach the code the same way they would reach any
  other server secret: on the machine.
- We are committed to the script being the only implementation of the install.
  A graphical road that re-implements step 2 or step 4 is the thing this ADR
  exists to prevent.

## Rejected

- **A signed native installer (`.pkg`, `.msi`).** The conventional comfortable
  road, and the one this ADR would have taken if it were free. Rejected on the
  owner's word after the cost was verified: roughly $220/year to sign both
  platforms, for a double-click that runs the script we already have.
- **Provisioning the container engine ourselves** (Colima or Lima on macOS,
  WSL2 on Windows). It is the only way to make the word "Docker" disappear, and
  it makes every engine failure ours. The walk already showed where those come
  from: a Docker Desktop VM woken from host sleep with a skewed clock (0.2.2
  shipped a detector for it), and a named volume vanishing under a running
  factory when an external disk was unplugged. Owning that lifecycle with a
  progress bar on top is a second product.
- **A Docker Desktop extension as the graphical road.** Free, genuinely
  graphical, and dead on arrival: Extensions are disabled by default since
  August 2026, so reaching it means a settings toggle first.
- **An installer written as a Rust binary.** The project is Rust and the
  release already builds artifacts, but a binary has to be fetched before it can
  run — which is `curl | sh` with an extra step, or a package we decided not to
  sign.
- **Pinning the compose file to a version inside the installer.** ADR-0043
  settled it: the asset names `latest` on purpose, because a pin would break the
  single line that both starts and updates.
