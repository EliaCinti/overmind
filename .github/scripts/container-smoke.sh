#!/usr/bin/env bash
#
# Does a day's work happen inside the image, and is the agent held while it does?
#
# Not "does the image build" — that has always been true, and it was true on
# every one of the four defects M19 opens with: no agent CLI in the image, no
# cage off macOS, a run that produces nothing reporting success anyway, and a
# container whose agents run as root. The weakest check that catches the first
# three is this one: start the container, open a task, and require a real file
# to come out of it.
#
# The fourth needs a pair. Since ADR-0029 the cage in the image is an
# unprivileged uid, and a denial only proves something if the identical run
# succeeds without it — otherwise a typo in the probe reads as security. So the
# same task runs twice, caged and with `OVERMIND_SANDBOX=off`, and the assertion
# is the *difference* between them.
#
# **What this deliberately does not prove.** The adapter here is a shell script,
# and a shell script writes freely — it has no permission system to be denied
# by. So this proves Overmind's plumbing (a task reaches an adapter, the
# adapter's files come back as artifacts, the run directory is writable) and the
# uid boundary around it, and says nothing about whether the *real* CLI can
# write. That is the trap this project already fell into once: "stubs are shell
# scripts and write freely, so every test was green over it" (M2 smoke run).

set -euo pipefail

IMAGE="${IMAGE:-overmind:ci}"
PORT="${PORT:-7070}"
API="http://127.0.0.1:${PORT}/api"
WORK="$(mktemp -d)"
NAME=""

cleanup() {
    if [ -n "$NAME" ]; then
        echo "--- container logs ($NAME) ---"
        docker logs "$NAME" 2>&1 | tail -40 || true
        docker rm -f "$NAME" >/dev/null 2>&1 || true
    fi
    # The code-task leg leaves git objects in the mounted repo owned by the
    # container's uids, which the runner's user cannot delete -- and a trap
    # that exits 1 turns a green run red after everything passed (measured:
    # the first CI run of the leg). Hand ownership back through the image,
    # and never let the tidy-up outrank the verdict.
    docker run --rm -v "$WORK:/w" --entrypoint chown "$IMAGE" -R "$(id -u):$(id -g)" /w         >/dev/null 2>&1 || true
    rm -rf "$WORK" || true
}
trap cleanup EXIT

# An adapter that does the one thing a knowledge task is for — leave a file
# behind — and, while it is in there, reports what it can reach. `id -u` and two
# probes at Overmind's own data: the database with its audit chain, and the
# directory holding every company's brain.
mkdir -p "$WORK/stub"
cat > "$WORK/stub/agent.sh" <<'STUB'
#!/bin/sh
printf 'The container can do a day of work.\n' > ARTIFACT.md
{
    echo "uid=$(id -u)"
    if [ -r /data/overmind.sqlite ]; then echo "db=READABLE"; else echo "db=DENIED"; fi
    if ls /data/companies >/dev/null 2>&1; then echo "brains=READABLE"; else echo "brains=DENIED"; fi
    if ls /data/backups >/dev/null 2>&1; then echo "backups=READABLE"; else echo "backups=DENIED"; fi
} > PROBE.txt
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
STUB
# The mount has to be reachable by a uid that is not the one that made it:
# `mktemp -d` gives 0700, which an unprivileged agent cannot traverse. Without
# this the caged run fails to *start* its adapter, which would look exactly like
# the boundary working and would be nothing of the kind.
chmod 755 "$WORK/stub" "$WORK/stub/agent.sh"

# A tiny real repository, for the code-task leg. Mounted read-write: a code
# run's worktree lives under /data, but git needs to reach the main repo.
mkdir -p "$WORK/repo"
git -C "$WORK/repo" init -q -b main
echo "# demo" > "$WORK/repo/README.md"
git -C "$WORK/repo" add . 
git -C "$WORK/repo" -c user.email=ci@ci -c user.name=CI commit -qm init
chmod -R a+rwX "$WORK/repo"

# Does the image ship an agent CLI, and can the agent actually run it?
#
# The first defect this milestone opens with is that it did not — the server's
# default adapter is `claude -p …`, so every task died at the spawn with
# `command not found`. This costs nothing to check and needs no credentials: an
# unauthenticated CLI still parses its arguments and reaches the API boundary,
# so the reply is `Not logged in`. Which is precisely the interesting answer,
# because it can only be reached by a CLI that exists, is runnable by this uid,
# and **was allowed the permission flag** — as root that flag is refused before
# authentication is ever attempted, and the reply is a refusal instead.
echo "checking the image's agent CLI…"
docker run --rm --user 10001:10001 -e HOME=/home/agent "$IMAGE" \
    timeout 60 claude -p hi --dangerously-skip-permissions --output-format json \
    > "$WORK/cli.txt" 2>&1 || true
python3 - "$WORK/cli.txt" <<'CHECK'
import sys
said = open(sys.argv[1]).read()
if "not found" in said:
    print("FAIL: the image ships no agent CLI — a task would die at the spawn.")
    print("     ", said.strip()[:200]); sys.exit(1)
if "root/sudo privileges" in said:
    print("FAIL: the CLI refused the permission flag, so this uid is root after all.")
    print("     ", said.strip()[:200]); sys.exit(1)
if "Not logged in" not in said:
    print("NOTE: the CLI answered something unexpected. Not fatal — it may be")
    print("      authenticated in this environment — but worth reading:")
    print("     ", said.strip()[:300])
else:
    print("  the CLI is present, runs as the agent, and was allowed the flag.")
CHECK

# The owner's session, once claimed. Every call below carries it: since
# ADR-0045 an instance nobody has claimed answers about its door and nothing
# else, so a smoke test that founds a company has to be somebody first.
COOKIE_JAR=""

api() { # method path [body]
    if [ $# -ge 3 ]; then
        curl -fsS -b "$COOKIE_JAR" -X "$1" "${API}$2" -H 'content-type: application/json' -d "$3"
    else
        curl -fsS -b "$COOKIE_JAR" -X "$1" "${API}$2"
    fi
}

# Claim the instance with the code it minted at first boot, the way the person
# at the machine does: read it out of the container's own data dir.
claim_instance() { # container-name label
    local name="$1" label="$2" code
    code=$(docker exec "$name" sh -c 'cat /data/setup-code 2>/dev/null' | tr -d '\r\n')
    if [ -z "$code" ]; then
        echo "[$label] the server minted no setup code — nothing can claim it"
        docker logs "$name" 2>&1 | tail -20
        exit 1
    fi
    COOKIE_JAR="$WORK/cookies-$label"
    curl -fsS -c "$COOKIE_JAR" -X POST "${API}/auth/claim" \
        -H 'content-type: application/json' \
        -d "{\"name\":\"ci\",\"password\":\"a long enough password\",\"setup\":\"$code\"}" \
        >/dev/null || { echo "[$label] the claim was refused"; exit 1; }
    echo "[$label] claimed with the setup code"
}

json() { python3 -c "import sys,json;print(json.load(sys.stdin)$1)"; }

# One full scenario: boot the image, found a company, run a knowledge task, and
# leave the artifacts in $WORK/<label>.json.
scenario() { # label [extra docker -e args…]
    local label="$1"; shift
    NAME="overmind-smoke-$$-${label}"

    # `OVERMIND_SANDBOX_ALLOW=/stub` is not scaffolding — it is the escape hatch
    # ADR-0023 shipped for exactly this, and the first thing in CI to exercise
    # it. Deny-by-default means an adapter somewhere the cage never heard of is
    # denied, and on a Landlock kernel that is enforced rather than assumed: the
    # first run of this script on a runner with Landlock failed here, with the
    # shell unable to read its own script. The *real* adapter needs nothing,
    # because it lives in /usr; a stub bind-mounted at the root does, and so
    # would anyone's custom `OVERMIND_AGENT_CMD`.
    docker run -d --name "$NAME" \
        -p "127.0.0.1:${PORT}:7070" \
        -v "$WORK/stub:/stub:ro" \
        -v "$WORK/repo:/repo" \
        -e OVERMIND_AGENT_CMD='sh /stub/agent.sh' \
        -e OVERMIND_SANDBOX_ALLOW=/stub \
        "$@" \
        "$IMAGE" >/dev/null

    echo "[$label] waiting for the server…"
    for _ in $(seq 1 60); do
        if curl -fsS "${API}/health" >/dev/null 2>&1; then break; fi
        sleep 1
    done
    curl -fsS "${API}/health" >/dev/null || { echo "the server never answered"; exit 1; }

    # The line the server prints about what is holding its agents. Worth showing:
    # when this pair disagrees with expectation, this is the first thing to read.
    docker logs "$NAME" 2>&1 | grep -i "agent confinement" || true

    claim_instance "$NAME" "$label"

    echo "[$label] founding a company…"
    local company company_id agent_id task task_id started session_id status
    company=$(api POST /companies '{"name":"CI"}')
    company_id=$(echo "$company" | json '["id"]')
    agent_id=$(echo "$company" | json '["ceo"]["id"]')

    # The founding memory, read back through the image's own memory provider
    # (M21, ADR-0031). No stub stands in here: the browse only answers `ok`
    # with an item if the real in-image Wadachi was spawned, stored the memory
    # at founding, and listed it back. This is the check that would have
    # caught "the image ships no memory provider" — the M19 acceptance run's
    # confident document about the wrong company.
    echo "[$label] asking the brain who the company is…"
    api GET "/companies/${company_id}/memory/memories" > "$WORK/brain-${label}.json"
    python3 - "$WORK/brain-${label}.json" <<'CHECK'
import json, sys
body = json.load(open(sys.argv[1]))
if body.get("state") != "ok":
    print("FAIL: the browse did not answer ok — the image's memory provider is not working:")
    print("     ", body)
    sys.exit(1)
titles = [i.get("title", "") for i in body.get("items", [])]
if "Who CI is" not in titles:
    print("FAIL: no founding memory in the brain — a fresh company starts empty again:", titles)
    sys.exit(1)
print("  the in-image brain holds the founding memory.")
CHECK

    echo "[$label] opening a task…"
    task=$(api POST "/companies/${company_id}/tasks" \
        '{"title":"Leave something behind","description":"Write ARTIFACT.md.","execution_kind":"knowledge"}')
    task_id=$(echo "$task" | json '["id"]')
    api POST "/tasks/${task_id}/transition" '{"to":"todo"}' >/dev/null
    started=$(api POST "/tasks/${task_id}/start" "{\"agent_id\":\"${agent_id}\"}")
    session_id=$(echo "$started" | json '["session_id"]')

    echo "[$label] waiting for the run…"
    for _ in $(seq 1 60); do
        status=$(api GET "/sessions/${session_id}" | json '.get("status","")')
        case "$status" in completed|failed) break ;; esac
        sleep 1
    done
    echo "[$label] session: ${status}"

    api GET "/tasks/${task_id}/artifacts" > "$WORK/${label}.json"

    # The code-task leg (M23). The knowledge task above never touches git, so
    # the smoke was green while every code task's diff in the image was
    # broken: the agent commits as uid 10001, the server asks `git diff` as
    # root, and git refuses a repository owned by another user. The check
    # that would have caught it is exactly this: run a code task and demand
    # the diff endpoint answers with the change in it.
    echo "[$label] a code task, and its diff…"
    local project_id workspace_ok code_task code_task_id code_started code_session code_status
    project_id=$(api POST "/companies/${company_id}/projects" '{"title":"P"}' | json '["id"]')
    api POST "/projects/${project_id}/workspaces" '{"name":"w","cwd":"/repo"}' >/dev/null
    code_task=$(api POST "/companies/${company_id}/tasks" \
        '{"title":"Change something","description":"Create NOTE.md with one line.","execution_kind":"code"}')
    code_task_id=$(echo "$code_task" | json '["id"]')
    api POST "/tasks/${code_task_id}/transition" '{"to":"todo"}' >/dev/null
    code_started=$(api POST "/tasks/${code_task_id}/start" "{\"agent_id\":\"${agent_id}\"}")
    code_session=$(echo "$code_started" | json '["session_id"]')
    for _ in $(seq 1 60); do
        code_status=$(api GET "/sessions/${code_session}" | json '.get("status","")')
        case "$code_status" in completed|failed) break ;; esac
        sleep 1
    done
    echo "[$label] code session: ${code_status}"
    api GET "/sessions/${code_session}/diff" > "$WORK/diff-${label}.txt" \
        || { echo "FAIL: the diff endpoint refused — a person cannot review the code task"; exit 1; }
    grep -q "ARTIFACT.md" "$WORK/diff-${label}.txt" \
        || { echo "FAIL: the diff came back without the run's change in it:"; head -5 "$WORK/diff-${label}.txt"; exit 1; }
    echo "  the diff shows the change."

    echo "[$label] verifying the audit chain…"
    # Via a file, not a pipe: the heredoc below *is* this python's stdin.
    api GET /audit/verify > "$WORK/audit-${label}.json"
    python3 - "$WORK/audit-${label}.json" <<'CHECK'
import json, sys
report = json.load(open(sys.argv[1]))
if not report.get("valid"):
    print("FAIL: the audit chain does not verify:", report)
    sys.exit(1)
print("  chain verifies over", report["events_checked"], "events.")
CHECK

    docker rm -f "$NAME" >/dev/null 2>&1 || true
    NAME=""
}

scenario caged
scenario uncaged -e OVERMIND_SANDBOX=off

# The assertions. Two of them, and neither is worth much without the other.
python3 - "$WORK/caged.json" "$WORK/uncaged.json" <<'CHECK'
import json, sys

def probe(path):
    artifacts = json.load(open(path))["artifacts"]
    files = {a["relative_path"]: (a.get("content") or "")
             for a in artifacts if a.get("relative_path")}
    if not files:
        print(f"FAIL: the run left no file behind ({path}).")
        print("      artifacts:", [a.get("title") for a in artifacts] or "none")
        print("      A `Run output` on its own is the adapter's transcript, not a deliverable.")
        sys.exit(1)
    if "day of work" not in files.get("ARTIFACT.md", ""):
        print(f"FAIL: the deliverable came back without what was written into it ({path}).")
        sys.exit(1)
    got = dict(line.split("=", 1) for line in files.get("PROBE.txt", "").split() if "=" in line)
    if not got:
        print(f"FAIL: the probe wrote nothing ({path}).")
        sys.exit(1)
    return got

caged, uncaged = probe(sys.argv[1]), probe(sys.argv[2])
print("  caged:  ", caged)
print("  uncaged:", uncaged)

fails = []

# The deliverable arrived in both — so the run works, and the boundary is not
# "the agent could not do anything".
# Caged: another uid than the server's, and Overmind's own data out of reach.
if caged["uid"] == "0":
    fails.append("the caged agent ran as root; ADR-0029's boundary is not in place "
                 "(and the real CLI refuses --dangerously-skip-permissions as root)")
if caged.get("db") != "DENIED":
    fails.append("the caged agent could read overmind.sqlite — the audit chain and "
                 "every company's data")
if caged.get("brains") != "DENIED":
    fails.append("the caged agent could read /data/companies — every company's brain")
if caged.get("backups") != "DENIED":
    fails.append("the caged agent could read /data/backups — an archive is the whole "
                 "instance, database and brains and all (ADR-0044)")

# Uncaged: the same probes must *succeed*, or the denials above prove nothing —
# a typo in the probe would read as security.
if uncaged["uid"] != "0":
    fails.append("the uncaged agent was not root, so the pair does not isolate the cage")
if (uncaged.get("db") != "READABLE" or uncaged.get("brains") != "READABLE"
        or uncaged.get("backups") != "READABLE"):
    fails.append("the uncaged agent could not read Overmind's data either, so the caged "
                 "denials say nothing about the cage")

if fails:
    for f in fails:
        print("FAIL:", f)
    sys.exit(1)

print("the container did a day's work, and the agent was held while it did.")
CHECK
