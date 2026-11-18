#!/bin/bash
set -u
M="${MIAUDIT_BIN:-$HOME/.cargo/bin/miaudit}"
UI="${MIAUDIT_UI_BIN:-$HOME/.cargo/bin/miaudit-ui}"
SRV="${MIAUDIT_SERVER_BIN:-$HOME/.cargo/bin/miaudit-server}"
pass=0
fail=0
check() {
  if [ "$2" = "0" ]; then pass=$((pass+1)); echo "PASS: $1"; else fail=$((fail+1)); echo "FAIL: $1"; fi
}
eq() {
  [ "$2" = "$3" ]; check "$1 (got: $2)" $?
}
jget() {
  python3 -c "import json,sys; d=json.load(sys.stdin)
for k in sys.argv[1].split('.'):
    d = d[int(k)] if isinstance(d, list) else d.get(k)
print('' if d is None else d)" "$1"
}

W=$(mktemp -d)
trap '[ -n "$UIPID" ] && { kill $UIPID 2>/dev/null; wait $UIPID 2>/dev/null; }; [ -n "$SRVPID" ] && { kill $SRVPID 2>/dev/null; wait $SRVPID 2>/dev/null; }; rm -rf "$W"' EXIT
UIPID=
SRVPID=
cd "$W" || exit 2
echo "binary: $M sha256=$(shasum -a 256 "$M" | cut -c1-16) ui=$(shasum -a 256 "$UI" | cut -c1-16)"
echo "workdir: $W"

git init -q . && git config user.name e2e && git config user.email e2e@example.invalid && git config commit.gpgsign false
$M init --json >/dev/null; eq "init exits 0" $? 0
[ "$(cat .miaudit/.gitignore)" = "*" ]; check "store ignores itself" $?
[ ! -e .audit ]; check "no legacy .audit directory" $?
[ -f .miaudit/PROJECT.md ]; check "init creates the project profile template" $?
$M project check --json > pc.out; eq "incomplete profile exits 4" $? 4
eq "profile state is incomplete" "$(jget result.state < pc.out)" incomplete
msg=$(printf '{"hook_event_name":"SessionStart"}' | $M session-start)
case "$msg" in *"ask the developer"*"staging"*) check "session-start asks the developer the questions" 0;; *) check "session-start asks the developer the questions" 1;; esac
python3 - <<'PY'
import re
t=open('.miaudit/PROJECT.md').read()
t=re.sub(r'\nTODO\n','\nanswered in e2e\n',t)
open('.miaudit/PROJECT.md','w').write(t)
PY
$M project check --json >/dev/null; eq "filled profile exits 0" $? 0
msg=$(printf '{}' | $M session-start)
case "$msg" in *"read .miaudit/PROJECT.md"*) check "session-start points to the profile once complete" 0;; *) check "session-start points to the profile once complete" 1;; esac
printf 'STAGING_API_KEY=sk_live_e2eSECRETvalue123456\n' >> .miaudit/PROJECT.md
out=$($M project check --json); rc=$?
eq "a secret in the profile exits 2" $rc 2
case "$out" in *e2eSECRETvalue*) check "the secret value is not echoed" 1;; *) check "the secret value is not echoed" 0;; esac
python3 - <<'PY'
t=open('.miaudit/PROJECT.md').read().replace('STAGING_API_KEY=sk_live_e2eSECRETvalue123456\n','')
open('.miaudit/PROJECT.md','w').write(t)
PY
$M project check --json >/dev/null; eq "profile is clean again after removing the secret" $? 0

PAYLOAD_PROMPT=$(printf '{"hook_event_name":"UserPromptSubmit","session_id":"s","cwd":"%s","prompt":"e2e prompt TOKEN-IN-PROMPT"}' "$W")
out=$(printf '%s' "$PAYLOAD_PROMPT" | $M capture); eq "capture UserPromptSubmit exits 0 with empty stdout" "$?:$out" "0:"

printf 'hello\n' > a.txt
PAYLOAD_WRITE=$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"Write","tool_input":{"file_path":"%s/a.txt","content":"NEVER-STORED"},"tool_response":{}}' "$W" "$W")
out=$(printf '%s' "$PAYLOAD_WRITE" | $M capture); eq "capture Write exits 0 with empty stdout" "$?:$out" "0:"

PAYLOAD_BASH=$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"Bash","tool_input":{"command":"SECRET=hunter2 cargo test"},"tool_response":{"interrupted":false}}' "$W")
out=$(printf '%s' "$PAYLOAD_BASH" | $M capture); eq "capture Bash exits 0 with empty stdout" "$?:$out" "0:"

printf 'two\n' > b.txt
PATCH='*** Begin Patch\n*** Add File: b.txt\n+two\n*** End Patch'
PAYLOAD_CODEX=$(printf '{"hook_event_name":"PostToolUse","session_id":"c","turn_id":"t","cwd":"%s","model":"m","tool_name":"apply_patch","tool_input":{"command":"%s"},"tool_response":{}}' "$W" "$PATCH")
out=$(printf '%s' "$PAYLOAD_CODEX" | $M capture --agent codex); eq "capture Codex apply_patch exits 0 with empty stdout" "$?:$out" "0:"

ev=$($M events --json)
eq "event chain is ok" "$(echo "$ev" | jget result.chain)" ok
eq "four events recorded (prompt, Write, Bash, Codex apply_patch)" "$(echo "$ev" | jget result.events_total)" 4
if grep -q -e hunter2 -e NEVER-STORED -e TOKEN-IN-PROMPT .miaudit/events.jsonl; then check "no secrets in the event log" 1; else check "no secrets in the event log" 0; fi
grep -q codex_hook .miaudit/events.jsonl; check "codex events are tagged codex_hook" $?

out=$(printf '{"hook_event_name":"Stop"}' | $M stop-check 2>&1 >/dev/null); rc=$?
eq "stop-check blocks once on unaudited edits (exit 2)" $rc 2
printf '%s' "$out" | grep -q "a.txt"; check "stop-check names the edited file" $?
printf '{"hook_event_name":"Stop","stop_hook_active":true}' | $M stop-check >/dev/null 2>&1; eq "stop-check lets the agent stop when already continued" $? 0

$M add a.txt b.txt --json >/dev/null; eq "add exits 0" $? 0
SID=$($M status --json | jget result.snapshot.id)
printf '{"schema_version":"1","description":"e2e task"}' > ctx.json
$M context add --file ctx.json --json >/dev/null; eq "context add exits 0" $? 0
printf '{"schema_version":"1","question":"q","choice":"c","alternatives":["x"],"rationale":"r","risks":[],"snapshot_id":"%s"}' "$SID" > dec.json
$M decision add --file dec.json --json >/dev/null; eq "decision add exits 0" $? 0
printf '{"schema_version":"1","classification":"internal","description":"e2e evidence","content":"ok"}' > evd.json
$M evidence add --file evd.json --json >/dev/null; eq "evidence add exits 0" $? 0

$M commit -m "e2e commit" --json > commit.out; eq "audit commit exits 0" $? 0
AID=$(jget result.audit_id < commit.out)
case "$AID" in audit_*) check "audit id has the audit_ prefix" 0;; *) check "audit id has the audit_ prefix" 1;; esac
vr=$($M verify "$AID" --json); eq "verify exits 0" $? 0
eq "integrity passes" "$(echo "$vr" | jget result.verified.integrity)" pass
echo "$vr" | grep -q observed_events_chain; check "verify checks the observed event chain" $?
show=$($M show "$AID" --json)
eq "observed evidence is attached" "$(echo "$show" | python3 -c "import json,sys;o=json.load(sys.stdin)['result']['objects']['evidence'];print(sorted(e['object']['provenance'] for e in o))")" "['observed', 'self_reported']"
eq "stop-check is quiet after the audit commit" "$(printf '{"hook_event_name":"Stop"}' | $M stop-check >/dev/null 2>&1; echo $?)" 0

if gh auth status >/dev/null 2>&1; then
  eq "author provenance is derived from gh" "$(echo "$show" | jget result.manifest.author.provenance)" derived
  [ -n "$(echo "$show" | jget result.manifest.author.github_login)" ]; check "author github login recorded ($(echo "$show" | jget result.manifest.author.github_login))" $?
  [ -n "$(echo "$show" | jget result.manifest.author.github_id)" ]; check "author numeric id recorded" $?
else
  echo "SKIP: gh is not authenticated, author is $(echo "$show" | jget result.manifest.author.provenance)"
fi

$M git-hook install --json >/dev/null; eq "git-hook install exits 0" $? 0
[ -x .git/hooks/post-commit ]; check "post-commit hook is executable" $?
git add a.txt b.txt && git commit -q -m "real git commit"
sleep 1
b=$($M bindings "$AID" --json)
eq "post-commit hook linked the audit commit" "$(echo "$b" | jget result.bindings.0.body.correspondence)" full
sha=$(git rev-parse HEAD)
eq "related finds the audit from the git commit" "$($M related "$sha" --json | jget result.audits.0.audit_id)" "$AID"

blob=$(python3 -c "import json,sys;print(json.load(sys.stdin)['result']['objects']['evidence'][0]['object']['body']['artifact']['digest'].split(':')[1])" <<< "$show")
cp ".miaudit/blobs/$blob" blob.bak
printf 'tampered' >> ".miaudit/blobs/$blob"
$M verify "$AID" --json >/dev/null; eq "verify detects a tampered blob (exit 2)" $? 2
cp blob.bak ".miaudit/blobs/$blob"
$M verify "$AID" --json >/dev/null; eq "verify passes again after restoring the blob" $? 0

cp .miaudit/events.jsonl events.bak
sed -i.tmp '2d' .miaudit/events.jsonl && rm -f .miaudit/events.jsonl.tmp
$M events --json >/dev/null; eq "events detects a deleted event (exit 2)" $? 2
cp events.bak .miaudit/events.jsonl

PORT=$(python3 -c "import socket;s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1])")
"$UI" --repo "$W" --port "$PORT" >/dev/null 2>&1 &
UIPID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do curl -s -o /dev/null "http://127.0.0.1:$PORT/" && break; sleep 0.3; done
eq "viewer index returns 200" "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/")" 200
VSLUG=$(curl -s "http://127.0.0.1:$PORT/api/v1/config" | jget repo)
eq "viewer reports local mode" "$(curl -s "http://127.0.0.1:$PORT/api/v1/config" | jget mode)" local
eq "viewer detail route returns the app shell (200)" "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/$VSLUG/audit/$AID")" 200
shell=$(curl -s "http://127.0.0.1:$PORT/")
echo "$shell" | grep -q 'id="root"'; check "viewer serves the React app shell" $?
echo "$shell" | grep -q '<script>'; [ $? -ne 0 ]; check "app shell has no inline script" $?
curl -s -D - -o /dev/null "http://127.0.0.1:$PORT/" | tr -d '\r' | grep -i '^content-security-policy:' | grep -q "script-src 'self'"; check "viewer sends a script-src 'self' CSP" $?
eq "viewer rejects a foreign Host header (403)" "$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "http://127.0.0.1:$PORT/")" 403
eq "viewer rejects POST (405)" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/")" 405
page=$(curl -s "http://127.0.0.1:$PORT/$VSLUG/api/audits/$AID/detail")
eq "viewer API returns the live verify result" "$(echo "$page" | jget verification.integrity)" pass
echo "$page" | grep -q "hook_file_edits"; check "viewer API returns the hook coverage row" $?
eq "viewer API returns the Git link" "$(echo "$page" | jget bindings.0.body.correspondence)" full

cap() { printf '%s' "$1" | $M capture; }
cap "$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"Bash","tool_input":{"command":"ls"},"tool_response":{"stdout":"E2E-OUTPUT-SECRET","exit_code":7}}' "$W")"
cap "$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"Read","tool_input":{"file_path":"%s/a.txt"},"tool_response":{}}' "$W" "$W")"
cap "$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"WebFetch","tool_input":{"url":"https://docs.example.com/p?token=E2E-URL-SECRET"},"tool_response":{}}' "$W")"
cap "$(printf '{"hook_event_name":"PostToolUse","session_id":"s","cwd":"%s","tool_name":"Task","tool_input":{"subagent_type":"reviewer","prompt":"E2E-PROMPT-SECRET"},"tool_response":{}}' "$W")"
cap "$(printf '{"hook_event_name":"SubagentStart","session_id":"s","agent_id":"ag-1","agent_type":"reviewer"}' )"
printf '{"hook_event_name":"Stop","session_id":"s","last_assistant_message":"E2E-REPLY-SECRET","stop_hook_active":true}' | $M stop-check >/dev/null 2>&1; eq "stop-check records the assistant message and lets the agent stop" $? 0
if grep -q -e E2E-OUTPUT-SECRET -e E2E-URL-SECRET -e E2E-PROMPT-SECRET -e E2E-REPLY-SECRET .miaudit/events.jsonl; then check "no output, URL query, prompt or reply text in the event log" 1; else check "no output, URL query, prompt or reply text in the event log" 0; fi
grep -q '"exit_code":7' .miaudit/events.jsonl; check "command exit code is recorded" $?
grep -q '"host":"docs.example.com"' .miaudit/events.jsonl; check "web fetch records only the host" $?
grep -q '"action":"spawn"' .miaudit/events.jsonl; check "subagent spawn is recorded" $?
grep -q '"kind":"assistant_message"' .miaudit/events.jsonl; check "assistant message digest is recorded" $?
$M events --json >/dev/null; eq "event chain still verifies" $? 0

printf 'ui\n' > c.txt
printf '{"schema_version":"1","description":"frontend task","change_kind":"frontend"}' > ctx2.json
$M add c.txt --json >/dev/null
$M context add --file ctx2.json --json >/dev/null
gaps=$($M gaps --json)
echo "$gaps" | grep -q "FRONTEND_EVIDENCE_MISSING:before"; check "frontend change asks for a before screenshot" $?
echo "$gaps" | grep -q "FRONTEND_EVIDENCE_MISSING:after"; check "frontend change asks for an after screenshot" $?
python3 -c "import sys;sys.stdout.buffer.write(bytes([0x89,0x50,0x4e,0x47,0x0d,0x0a,0x1a,0x0a])+b'e2e-pixels')" > shot.png
printf '{"schema_version":"1","classification":"internal","description":"UI before the change","file":"shot.png","phase":"before","environment":"local"}' > shot-before.json
printf '{"schema_version":"1","classification":"internal","description":"UI after the change","file":"shot.png","phase":"after","environment":"staging"}' > shot-after.json
$M evidence add --file shot-before.json --json >/dev/null; eq "screenshot before added" $? 0
$M evidence add --file shot-after.json --json >/dev/null; eq "screenshot after added" $? 0
gaps=$($M gaps --json)
echo "$gaps" | grep -q "FRONTEND_EVIDENCE_MISSING"; [ $? -ne 0 ]; check "screenshots satisfy the frontend evidence gaps" $?
SID2=$($M status --json | jget result.snapshot.id)
printf '{"schema_version":"1","question":"q2","choice":"c","alternatives":["x"],"rationale":"r","risks":[],"snapshot_id":"%s"}' "$SID2" > dec2.json
$M decision add --file dec2.json --json >/dev/null
$M commit -m "frontend change" --json > commit2.out; eq "frontend audit commit exits 0" $? 0
AID2=$(jget result.audit_id < commit2.out)
cov=$($M show "$AID2" --json | python3 -c "import json,sys;m=json.load(sys.stdin)['result']['manifest'];print(sorted(c['capability'] for c in m['coverage'] if c['provenance']=='observed' and c['capability']!='hook_file_edits'))")
eq "all four capture capabilities are observed" "$cov" "['agent_topology', 'command_execution_receipts', 'conversation_capture', 'resource_access_receipts']"
$M show "$AID2" --json | grep -q "CAPTURE_PARTIAL:resource_access_receipts"; check "gaps say the capture is partial" $?
$M verify "$AID2" --json >/dev/null; eq "verify passes the audit with the new events" $? 0
EID=$($M show "$AID2" --json | python3 -c "import json,sys;print([e['id'] for e in json.load(sys.stdin)['result']['objects']['evidence'] if e['object']['body'].get('phase')=='before'][0])")
code=$(curl -s -o img.out -w '%{http_code}:%{content_type}' "http://127.0.0.1:$PORT/$VSLUG/api/audits/$AID2/evidence/$EID/image")
eq "viewer serves the screenshot as image/png" "$code" "200:image/png"
cmp -s img.out shot.png; check "served screenshot is byte-identical to the attached file" $?
eq "viewer API flags the screenshot as an image" "$(curl -s "http://127.0.0.1:$PORT/$VSLUG/api/audits/$AID2/detail" | python3 -c "import json,sys;print(any(e['image'] for e in json.load(sys.stdin)['evidence']))")" True

if [ -n "${MIAUDIT_TEST_MONGO_URI:-}" ] && [ -x "$SRV" ]; then
  SPORT=$(python3 -c "import socket;s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1])")
  SDB="miaudit_e2e_$$_$RANDOM"
  export MIAUDIT_MONGO_URI="$MIAUDIT_TEST_MONGO_URI" MIAUDIT_MONGO_DB="$SDB"
  TOKEN=$($SRV token create e2e 2>/dev/null); eq "server creates a push token" $? 0
  case "$TOKEN" in mdt_*) check "token has the mdt_ prefix" 0;; *) check "token has the mdt_ prefix" 1;; esac
  $SRV serve --port "$SPORT" > server.log 2>&1 &
  SRVPID=$!
  for i in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$SPORT/healthz" && break; sleep 0.1; done
  MIAUDIT_SERVER="http://127.0.0.1:$SPORT" MIAUDIT_TOKEN="wrong" $M push --json > push-bad.out; eq "push with a wrong token exits 2" $? 2
  eq "wrong token is reported as PUSH_UNAUTHORIZED" "$(jget error.code < push-bad.out)" PUSH_UNAUTHORIZED
  MIAUDIT_SERVER="http://127.0.0.1:$SPORT" MIAUDIT_TOKEN="$TOKEN" $M push --json > push1.out; eq "push exits 0" $? 0
  OFFERED=$(jget result.offered.commits < push1.out)
  eq "first push sends every audit commit" "$(jget result.sent.commits < push1.out)" "$OFFERED"
  MIAUDIT_SERVER="http://127.0.0.1:$SPORT" MIAUDIT_TOKEN="$TOKEN" $M push --json > push2.out; eq "second push exits 0" $? 0
  eq "second push sends no commits" "$(jget result.sent.commits < push2.out)" 0
  eq "second push sends no blobs" "$(jget result.sent.blobs < push2.out)" 0
  SLUG=$(jget result.repo < push1.out)
  curl -s "http://127.0.0.1:$SPORT/api/v1/repos" > repos.out
  eq "server lists the repo with its audit count" "$(jget repos.0.audits < repos.out)" "$OFFERED"
  eq "server lists the pushed slug" "$(jget repos.0.slug < repos.out)" "$SLUG"
  curl -s "http://127.0.0.1:$SPORT/" | grep -q 'id="root"'; check "server serves the React app shell" $?
  eq "server reports server mode" "$(curl -s "http://127.0.0.1:$SPORT/api/v1/config" | jget mode)" server
  curl -s "http://127.0.0.1:$SPORT/$SLUG/api/audits" | grep -q "frontend change"; check "repo audit list API returns the audit commits" $?
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$SPORT/$SLUG/audit/$AID2")
  eq "audit route opens under the repo (app shell)" "$code" 200
  eq "server-side integrity check passes" "$(curl -s "http://127.0.0.1:$SPORT/$SLUG/api/audits/$AID2/detail" | jget verification.integrity)" pass
  code=$(curl -s -o img2.out -w '%{http_code}:%{content_type}' "http://127.0.0.1:$SPORT/$SLUG/api/audits/$AID2/evidence/$EID/image")
  eq "server serves the screenshot as image/png" "$code" "200:image/png"
  cmp -s img2.out shot.png; check "server screenshot is byte-identical to the attached file" $?
  curl -s "http://127.0.0.1:$SPORT/$SLUG/api/audits/$AID2" > raw.out
  cmp -s raw.out ".miaudit/commits/$AID2.json"; check "server returns the exact manifest bytes" $?
  code=$(curl -s -o /dev/null -w '%{http_code}' -X PUT -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' --data '{"x":1}' "http://127.0.0.1:$SPORT/api/v1/repos/$(jget result.repo_id < push1.out)/commits/audit_$(printf '0%.0s' $(seq 1 64))")
  eq "server refuses a commit whose content does not hash to its id" "$code" 422
  $M remote --server "http://127.0.0.1:$SPORT" repos --json > qrepos.out; eq "remote repos exits 0" $? 0
  eq "remote repos lists the pushed slug" "$(jget result.repos.0.slug < qrepos.out)" "$SLUG"
  $M remote --server "http://127.0.0.1:$SPORT" audits "$SLUG" --gap NO_HUMAN_APPROVAL --json > qaudits.out; eq "remote audits exits 0" $? 0
  eq "every audit has the no-human-approval gap" "$(jget result.total < qaudits.out)" "$OFFERED"
  $M remote --server "http://127.0.0.1:$SPORT" audits "$SLUG" --gap NO_SUCH_GAP_CODE --json > qnone.out
  eq "an unmatched gap filter returns nothing" "$(jget result.total < qnone.out)" 0
  $M remote --server "http://127.0.0.1:$SPORT" audits "$SLUG" | grep -q "audit commit(s) match in $SLUG"; check "remote audits prints a readable table without --json" $?
  $M remote --server "http://127.0.0.1:$SPORT" show "$SLUG" "$AID2" --json > qshow.out; eq "remote show exits 0" $? 0
  eq "remote show returns the manifest description" "$(jget result.manifest.description < qshow.out)" "frontend change"
  $SRV token revoke e2e >/dev/null 2>&1
  MIAUDIT_SERVER="http://127.0.0.1:$SPORT" MIAUDIT_TOKEN="$TOKEN" $M push --json > push3.out; eq "revoked token cannot push (exit 2)" $? 2
  unset MIAUDIT_MONGO_URI MIAUDIT_MONGO_DB
else
  echo "SKIP: server section (set MIAUDIT_TEST_MONGO_URI and install miaudit-server to run it)"
fi

echo "e2e: $pass passed, $fail failed"
[ "$fail" = "0" ]
