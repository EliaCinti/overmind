# ADR-0045: The install is one script, and an unclaimed instance is not open

- **Date:** 2026-09-01
- **Status:** proposed
- **Builds on:** [ADR-0043](0043-the-compose-file-is-the-installers.md) (the compose file is the installer's, and the asset is where you get it), [ADR-0014](0014-docker-deployment.md) (one deployment image), [ADR-0032](0032-authentication-the-boundary-moves-off-the-machine.md) (the first run claims the instance), [ADR-0044](0044-the-archive-is-the-instance.md) (a restore is a claim with a payload).

## Context

Installing Overmind today is five things a person does by hand: get a container
engine, download the compose file, export a credential, run
`docker compose up -d --pull always`, then open a browser and claim the
instance before anybody else does. ADR-0043 removed the worst of it — no clone,
one file, one command that both starts and updates. What is left is a procedure
a person reads and performs, and two of its steps are the ones that bit on the
two-machine walk: the engine was never covered, and *"claim it before anyone
else"* is advice, not a mechanism.

The second one is not a rough edge, it is a hole, and it is wider than the
claim. `auth::wall` says so in its own comment — *"No owner yet: the whole API
stays open, exactly as before M24"* — and `require_owner` returns `Ok` in the
same state. On an unclaimed instance a stranger who reaches the port does not
need to claim anything: they can found a company and start a task, which runs
an agent CLI on the host; call `/claude-auth/start` and `/claude-auth/code`;
set `/economy/pay-with`; mint `/auth/invites`; or `POST /api/restore` their own
archive over it. On loopback this is theoretical. The moment somebody binds
Overmind to a tailnet address — which the wiki teaches, because sharing a
company is the product — it is the whole instance, not just the owner account.

The owner's constraint on the second road, given plainly: it **must cost
nothing**. Signing was priced before it was rejected — Apple Developer at
$99/year for notarization, Azure Trusted Signing at $9.99/month for Windows, an
EV certificate at $280+/year — and an unsigned `.pkg` or `.msi` greets a person
with the warning macOS and Windows reserve for malware, which is worse than the
terminal it was meant to replace.

## Decision

1. **One script is the installer**, in the repository at `scripts/install.sh`
   (POSIX `sh`, macOS and Linux) with a Windows twin `scripts/install.ps1`. It
   does seven things and nothing else:

   1. names the OS and architecture, and refuses what it cannot install with a
      sentence rather than an error;
   2. **asks the engine whether it is there, not the filesystem.** The check is
      `docker info`, because the two failures that actually happen both pass a
      `command -v docker`: Docker Desktop installed but not started, and a user
      not yet in the `docker` group after `get.docker.com` (which needs a new
      login). Three outcomes, three different sentences — *not installed* prints
      the one line for that platform, *installed but not running* says to start
      it, *running but not reachable by you* says to log out and back in — and
      **none of them installs anything**. A prerequisite stated is not the same
      thing as a system changed behind someone's back;
   3. creates its directory: **`./overmind`, beside whoever ran the command**,
      unless `OVERMIND_HOME` names somewhere else.

      *Corrected 3 Sep 2026, twice over.* This ADR described
      `$XDG_DATA_HOME/overmind` falling back to `~/.overmind`; the script has
      never done that — ADR-0047 made the folder itself the installation, and
      the code went to `$HOME/overmind` without this line following it. It is
      now `./overmind`, because the owner ran the installer from his Desktop
      and went looking there for what it made. A subfolder rather than the
      bare working directory: scattering a compose file, a `data/` and an
      `agent/` into whatever directory somebody happened to be in is not a
      gift.

      That makes a *new* mistake reachable, and the script refuses it rather
      than letting it happen: since a folder **is** an instance, installing
      into a fresh one beside an existing Overmind does not update that
      Overmind, it stands an empty one next to it and leaves the real database
      where nobody is looking. So when the directory was defaulted rather than
      chosen, and an instance exists at the previous default, the installer
      stops and gives three lines — update the one you have, move it here, or
      set `OVERMIND_HOME` and mean it. An explicit choice is never second-
      guessed;
   4. downloads the compose file **from its own release, by tag, and checks it
      against the SHA-256 the script carries**. The script is stamped at release
      time with its release's tag and that asset's digest, so the file and its
      expected hash come from one place and no release cut can land between two
      requests. Per ADR-0043 the asset's *content* still names `latest` — the
      pin is on the URL, never on `image:`, and the one update line keeps
      working;
   5. runs `docker compose up -d --pull always`;
   6. waits for the server to answer, then prints the URL and **the setup code**
      (below), read from the file the server wrote — never scraped from the log,
      which on the second run still holds the first run's line;
   7. closes by saying the three things that matter next: that this same command
      is also the update, that **the agent still needs a way to pay** — the
      sign-in from the product's first screen, or `ANTHROPIC_API_KEY` exported
      before the run, which the script forwards if it is already set but cannot
      ask for, because under `curl … | sh` stdin is the pipe and not a terminal
      — and where the data lives, with the archive as its way out
      (`/wiki/backup`).

   The whole script is a function called on its last line, so a truncated
   download executes nothing. It writes only inside its directory. It does
   **not** refuse to run as root: on a fresh VPS or LXC root is often the only
   account, and on a just-installed Docker the non-root user cannot reach the
   socket until a new login. It refuses to run under `sudo` when the invoking
   user could have reached the engine themselves, which is the case the refusal
   was for.

2. **`https://overmind.eliacinti.dev/install.sh` is a real 30x with a
   `Location`**, served ahead of the site's SPA fallback, pointing at the
   release asset. This is a requirement and not a detail: the site is a
   single-page app behind `try_files … /index.html`, so a catch-all would answer
   `/install.sh` with the landing page and status 200, and `curl -fsSL … | sh`
   would pipe HTML into a shell. An HTML meta-refresh is not acceptable either —
   `curl -L` follows 3xx, not markup. The convenient form is
   `curl -fsSL https://overmind.eliacinti.dev/install.sh | sh`
   (`irm … /install.ps1 | iex` on Windows); the careful form, shown beside it,
   **pins a tag**: download `install.sh` and `install.sh.sha256` from a named
   release, check, read, run. Said honestly on the site: the convenient form is
   trust on first use, as every one-liner is; only the careful form gives
   provenance.

3. **The setup code is always required to claim, and the wall closes with it.**
   Two halves, and neither is worth anything alone:

   - **The code.** At first boot with no owner, the server mints a single-use
     code and writes it to `<data>/setup-code` (`0600`, the server's alone),
     plus one line in the log. It is a *file* and not a log scrape so that it
     survives a restart unchanged, can be read by the installer, and is not
     re-minted under a person who already copied it. The claim demands it; a
     successful claim deletes it. `POST /api/restore` demands it too — a restore
     *is* a claim (ADR-0044), and an archive from a stranger is a claim from a
     stranger.
   - **The wall.** While no owner exists, `auth::wall` stops waving the whole
     API through. Only the routes a fresh instance genuinely needs before it has
     an owner stay reachable — the claim, the restore, and what the door itself
     renders — each of them holding the code. Everything else answers `401`
     rather than running an agent CLI for a passer-by. This is the debt in
     `docs/NEXT.md` under *Small debts from the first live walk*; it is **not**
     deferred here, because decision 3's first half without its second is a
     security claim that is not true.

   There is deliberately no "loopback is different" branch. `Dockerfile:147`
   sets `OVERMIND_ADDR=0.0.0.0:7070` unconditionally, so inside the container
   the bind address is `0.0.0.0` whether the host publishes `127.0.0.1:7070:7070`
   or a tailnet address; the server cannot see the distinction, and a rule keyed
   on something it cannot observe would either demand a code from every Docker
   install or protect none of them. The code costs a localhost user nothing when
   they used the installer — it is printed in the terminal they are already
   looking at — and one `cat` of a named file when they did not.

4. **The graphical road comes second, wraps this same script, and costs
   nothing.** Never a second implementation of these seven steps. The
   candidates, all free, are written down so the choice is made on evidence
   rather than taste: a Homebrew cask and a `winget`/Scoop manifest
   (conventional and free, still a typed command); a double-clickable wrapper of
   the same script (free, and on macOS it needs one right-click → Open, because
   a downloaded file is quarantined); a Docker Desktop extension (free, but
   Extensions ship **disabled by default** since August 2026, so nobody would
   find it); and the home-server app stores — Umbrel, CasaOS, Unraid, a
   Portainer template — genuinely one click and free, for an audience Overmind
   has not yet met. Decided when we know who actually installs it.

## Consequences

- The install becomes one line to read out loud, the same on three platforms.
  The per-OS tabs on the landing stay, because the *prerequisite* differs per
  platform even when the install does not.
- The release workflow gains three assets — `install.sh`, `install.ps1`, and
  their checksums — and a release-time stamping step that writes the tag and the
  compose digest into the scripts. This depends on a release that carries them:
  today `releases/latest/download/docker-compose.yml` answers **404**, because
  `files:` entered `release.yml` after `v0.2.2` was tagged. **0.2.3 is the first
  release the installer can point at, and the script cannot ship before it.**
- **An unclaimed instance stops being an open API.** That is a behaviour change
  for anyone who scripted against a fresh instance before claiming it, and it
  wants its own row in `docs/THREAT-MODEL.md` with the tests that hold it: a
  claim without the code refused, a restore without the code refused, an
  agent-starting route refused before an owner exists, the code single-use, the
  file gone after the claim.
- A new supply-chain surface: a script executed from a URL. It is bounded the
  way the ecosystem bounds it — a release asset rather than a branch, a digest
  the script carries for what it downloads next, a careful form beside the
  convenient one, and a body that does nothing if truncated.
- We are committed to the script being the only implementation of the install. A
  graphical road that re-implements step 2 or step 4 is the thing this ADR
  exists to prevent.

## Rejected

- **A signed native installer (`.pkg`, `.msi`).** The conventional comfortable
  road, and the one this ADR would have taken if it were free. Rejected on the
  owner's word after the cost was verified: roughly $220/year to sign both
  platforms, for a double-click that runs the script we already have.
- **Gating the setup code on "the instance is not on loopback."** The first
  draft did this. The server cannot observe it — see decision 3 — so the rule
  would have fired on every default Docker install while promising it fired on
  none.
- **Shipping the setup code without closing the wall.** It would have read as
  security while leaving every pre-owner route open, which is worse than
  leaving the hole visible.
- **Prompting for a credential during the install.** Under `curl … | sh` stdin
  is the pipe; a prompt would either hang or silently read the script's own
  bytes. The sign-in belongs to the product's first screen, where it already is.
- **Provisioning the container engine ourselves** (Colima or Lima on macOS,
  WSL2 on Windows). The only way to make the word "Docker" disappear, and it
  makes every engine failure ours. The walk already showed where those come
  from: a Docker Desktop VM woken from host sleep with a skewed clock (0.2.2
  shipped a detector for it), and a named volume vanishing under a running
  factory when an external disk was unplugged.
- **A Docker Desktop extension as the graphical road.** Free, genuinely
  graphical, and dead on arrival: Extensions are disabled by default since
  August 2026, so reaching it means a settings toggle first.
- **An installer written as a Rust binary.** A binary has to be fetched before
  it can run — which is `curl | sh` with an extra step, or a package we decided
  not to sign.
- **Pinning the compose file's `image:` to a version.** ADR-0043 settled it: the
  asset names `latest` on purpose, because a pin would break the single line
  that both starts and updates. The *URL* is pinned here; the content is not.
