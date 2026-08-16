#!/usr/bin/env bash
#
# Does a day's work happen inside the image?
#
# Not "does the image build" — that has always been true, and it was true on
# every one of the three defects M19 opens with: no agent CLI in the image, no
# cage off macOS, and a run that produces nothing reporting success anyway.
# The weakest check that catches them is this one: start the container, open a
# task, and require a real file to come out of it.
#
# **What this deliberately does not prove.** The adapter here is a shell script,
# and a shell script writes freely — it has no permission system to be denied
# by. So this proves Overmind's plumbing (a task reaches an adapter, the
# adapter's files come back as artifacts, the run directory is writable) and
# says nothing about whether the *real* CLI can write, which depends on the
# cage. That is the trap this project already fell into once: "stubs are shell
# scripts and write freely, so every test was green over it" (M2 smoke run).
# The cage on Linux is what closes that half, and it is checked by its own
# paired probes, not here.

set -euo pipefail

IMAGE="${IMAGE:-overmind:ci}"
NAME="overmind-smoke-$$"
PORT="${PORT:-7070}"
API="http://127.0.0.1:${PORT}/api"
WORK="$(mktemp -d)"

cleanup() {
    echo "--- container logs ---"
    docker logs "$NAME" 2>&1 | tail -40 || true
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT

# An adapter that does the one thing a knowledge task is for: leave a file
# behind, and report what it cost.
mkdir -p "$WORK/stub"
cat > "$WORK/stub/agent.sh" <<'STUB'
#!/bin/sh
printf 'The container can do a day of work.\n' > ARTIFACT.md
echo '{"total_cost_usd":0.01,"model":"stub","usage":{"input_tokens":1,"output_tokens":1}}'
STUB

docker run -d --name "$NAME" \
    -p "127.0.0.1:${PORT}:7070" \
    -v "$WORK/stub:/stub:ro" \
    -e OVERMIND_AGENT_CMD='sh /stub/agent.sh' \
    "$IMAGE" >/dev/null

echo "waiting for the server…"
for _ in $(seq 1 60); do
    if curl -fsS "${API}/health" >/dev/null 2>&1; then break; fi
    sleep 1
done
curl -fsS "${API}/health" >/dev/null || { echo "the server never answered"; exit 1; }

api() { # method path [body]
    if [ $# -ge 3 ]; then
        curl -fsS -X "$1" "${API}$2" -H 'content-type: application/json' -d "$3"
    else
        curl -fsS -X "$1" "${API}$2"
    fi
}

echo "founding a company…"
COMPANY=$(api POST /companies '{"name":"CI"}')
COMPANY_ID=$(echo "$COMPANY" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
AGENT_ID=$(echo "$COMPANY" | python3 -c 'import sys,json;print(json.load(sys.stdin)["ceo"]["id"])')

echo "opening a task…"
TASK=$(api POST "/companies/${COMPANY_ID}/tasks" \
    '{"title":"Leave something behind","description":"Write ARTIFACT.md.","execution_kind":"knowledge"}')
TASK_ID=$(echo "$TASK" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
api POST "/tasks/${TASK_ID}/transition" '{"to":"todo"}' >/dev/null
STARTED=$(api POST "/tasks/${TASK_ID}/start" "{\"agent_id\":\"${AGENT_ID}\"}")
SESSION_ID=$(echo "$STARTED" | python3 -c 'import sys,json;print(json.load(sys.stdin)["session_id"])')

echo "waiting for the run…"
for _ in $(seq 1 60); do
    STATUS=$(api GET "/sessions/${SESSION_ID}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("status",""))')
    case "$STATUS" in completed|failed) break ;; esac
    sleep 1
done
echo "session: ${STATUS}"

# The assertion that matters, and the one nothing weaker would make: a file the
# agent wrote, not the `Run output` fallback. That fallback has no
# `relative_path` — it is Overmind's own transcript, and a run that delivers
# only it delivered nothing.
api GET "/tasks/${TASK_ID}/artifacts" > "$WORK/artifacts.json"
python3 - "$WORK/artifacts.json" <<'CHECK'
import json, sys

artifacts = json.load(open(sys.argv[1]))["artifacts"]
files = [a for a in artifacts if a.get("relative_path")]
if not files:
    print("FAIL: the run left no file behind.")
    print("      artifacts:", [a.get("title") for a in artifacts] or "none")
    print("      A `Run output` on its own is the adapter's transcript, not a deliverable.")
    sys.exit(1)
for a in files:
    print(f"  {a['relative_path']}  ({a['size_bytes']} bytes)")
    if "day of work" not in (a.get("content") or ""):
        print("FAIL: the file came back, but not with what was written into it.")
        sys.exit(1)
print("the container did a day's work.")
CHECK

echo "verifying the audit chain…"
api GET /audit/verify > "$WORK/audit.json"
python3 - "$WORK/audit.json" <<'CHECK'
import json, sys

report = json.load(open(sys.argv[1]))
if not report.get("valid"):
    print("FAIL: the audit chain does not verify:", report)
    sys.exit(1)
print("  chain verifies over", report["events_checked"], "events.")
CHECK
