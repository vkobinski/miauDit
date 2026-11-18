use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_miaudit");

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(BIN).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn hook_env(dir: &Path, env: &[(&str, &str)], sub: &str, input: &Value) -> (i32, String) {
    let mut cmd = Command::new(BIN);
    cmd.current_dir(dir).arg(sub).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut c = cmd.spawn().unwrap();
    c.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout), "", "hooks must not print to stdout");
    (o.status.code().unwrap(), String::from_utf8_lossy(&o.stderr).into())
}

fn stop(d: &Path, again: bool) -> (i32, String) {
    hook_env(d, &[], "stop-check", &json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": d, "stop_hook_active": again, "last_assistant_message": "done"}))
}

fn prompt(d: &Path, text: &str) {
    assert_eq!(hook_env(d, &[], "capture", &json!({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "cwd": d, "prompt": text})).0, 0);
}

fn bash(d: &Path, command: &str) {
    assert_eq!(hook_env(d, &[], "capture", &json!({"hook_event_name": "PostToolUse", "session_id": "s1", "cwd": d, "tool_name": "Bash", "tool_input": {"command": command}, "tool_response": {"stdout": "SECRET-OUTPUT", "exit_code": 0}})).0, 0);
}

fn edit(d: &Path, rel: &str) {
    assert_eq!(hook_env(d, &[], "capture", &json!({"hook_event_name": "PostToolUse", "session_id": "s1", "cwd": d, "tool_name": "Write", "tool_input": {"file_path": d.join(rel), "content": "x"}, "tool_response": {}})).0, 0);
}

fn repo_with_first_audit() -> (TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init", "--no-hooks"]).0, 0);
    fs::write(d.path().join("a.txt"), "one").unwrap();
    assert_eq!(audit(d.path(), &["add", "a.txt"]).0, 0);
    let (code, v) = audit(d.path(), &["commit", "-m", "first"]);
    assert_eq!(code, 0, "{v}");
    let id = v["result"]["audit_id"].as_str().unwrap().to_string();
    (d, id)
}

fn log(d: &Path) -> Vec<Value> {
    audit(d, &["log"]).1["result"]["commits"].as_array().unwrap().clone()
}

fn head(d: &Path) -> String {
    log(d)[0]["audit_id"].as_str().unwrap().to_string()
}

fn checkpoints(d: &Path, id: &str) -> Vec<Value> {
    audit(d, &["show", id]).1["result"]["checkpoints"].as_array().unwrap().clone()
}

fn manifest(d: &Path, id: &str) -> Value {
    audit(d, &["show", id]).1["result"]["manifest"].clone()
}

fn manifest_bytes(d: &Path, id: &str) -> Vec<u8> {
    fs::read(d.join(".miaudit/commits").join(format!("{id}.json"))).unwrap()
}

fn verify(d: &Path, id: &str) -> Value {
    audit(d, &["verify", id]).1["result"].clone()
}

fn check<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v["checks"].as_array().unwrap().iter().find(|c| c["check"] == name)
}

#[test]
fn a_turn_with_only_commands_and_conversation_is_attached_to_the_audit_without_changing_it() {
    let (d, first) = repo_with_first_audit();
    let before = manifest_bytes(d.path(), &first);
    prompt(d.path(), "rotate the keys");
    bash(d.path(), "bao kv put secret/x a=1");
    let (code, err) = stop(d.path(), false);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("checkpoint chk_"), "{err}");

    assert_eq!(log(d.path()).len(), 1, "no new audit commit is created");
    assert_eq!(head(d.path()), first);
    assert_eq!(manifest_bytes(d.path(), &first), before, "the audit commit, and so its hash, is untouched");

    let cps = checkpoints(d.path(), &first);
    assert_eq!(cps.len(), 1);
    let c = &cps[0];
    assert_eq!(c["body"]["audit_id"], first.as_str());
    assert_eq!(c["body"]["previous"], Value::Null);
    assert_eq!(c["body"]["provenance"], "observed");
    assert_eq!(c["body"]["summary"], json!({"prompts": 1, "assistant_messages": 1, "commands": 1, "resources": 0, "agent_events": 0}));
    assert_eq!(c["body"]["events_count"], 3);
    assert!(c["checkpoint_id"].as_str().unwrap().starts_with("chk_"));

    assert_eq!(log(d.path())[0]["checkpoints"], 1);
    assert_eq!(audit(d.path(), &["events"]).1["result"]["events_since_last_audit_commit"], 0, "the cursor advanced");
    let v = verify(d.path(), &first);
    assert_eq!(v["verified"]["integrity"], "pass");
    assert_eq!(check(&v, "checkpoints_integrity").unwrap()["result"], "pass", "{v}");

    let stored = fs::read_dir(d.path().join(".miaudit/blobs")).unwrap().map(|e| fs::read(e.unwrap().path()).unwrap()).map(|b| String::from_utf8_lossy(&b).to_string()).collect::<String>();
    assert!(!stored.contains("SECRET-OUTPUT") && !stored.contains("bao kv put"), "only digests are kept for commands");
}

#[test]
fn every_turn_adds_to_the_same_audit_as_a_linked_chain_that_verifies() {
    let (d, first) = repo_with_first_audit();
    for turn in 0..3 {
        prompt(d.path(), &format!("question {turn}"));
        bash(d.path(), &format!("kubectl get pods {turn}"));
        assert_eq!(stop(d.path(), false).0, 0);
    }
    assert_eq!(log(d.path()).len(), 1, "still one audit commit");
    let cps = checkpoints(d.path(), &first);
    assert_eq!(cps.len(), 3);
    assert_eq!(cps[0]["body"]["previous"], Value::Null);
    assert_eq!(cps[1]["body"]["previous"], cps[0]["checkpoint_id"]);
    assert_eq!(cps[2]["body"]["previous"], cps[1]["checkpoint_id"]);
    assert_eq!(cps[1]["body"]["first_prev"], cps[0]["body"]["last_event"], "the event chain continues across checkpoints");
    assert!(cps[1]["body"]["first_seq"].as_u64().unwrap() == cps[0]["body"]["last_seq"].as_u64().unwrap() + 1);
    let v = verify(d.path(), &first);
    assert_eq!(v["verified"]["integrity"], "pass");
    assert!(check(&v, "checkpoints_integrity").unwrap()["detail"].as_str().unwrap().starts_with("3 checkpoint(s)"));
}

#[test]
fn tampering_with_an_attached_checkpoint_is_detected() {
    let (d, first) = repo_with_first_audit();
    prompt(d.path(), "p");
    assert_eq!(stop(d.path(), false).0, 0);
    let cps = checkpoints(d.path(), &first);
    let cp_id = cps[0]["checkpoint_id"].as_str().unwrap().to_string();
    let file = d.path().join(".miaudit/checkpoints").join(format!("{cp_id}.json"));
    let original = fs::read_to_string(&file).unwrap();

    fs::write(&file, original.replace("\"prompts\": 1", "\"prompts\": 9")).unwrap();
    let v = verify(d.path(), &first);
    assert_eq!(v["verified"]["integrity"], "fail", "{v}");
    assert_eq!(check(&v, "checkpoints_integrity").unwrap()["result"], "fail");
    fs::write(&file, &original).unwrap();
    assert_eq!(verify(d.path(), &first)["verified"]["integrity"], "pass");

    let digest = cps[0]["body"]["events_blob"].as_str().unwrap().trim_start_matches("sha256:").to_string();
    let blob = d.path().join(".miaudit/blobs").join(&digest);
    let text = fs::read_to_string(&blob).unwrap();
    fs::write(&blob, text.replacen("assistant_message", "assistant_messagX", 1)).unwrap();
    let v = verify(d.path(), &first);
    assert_ne!(v["verified"]["integrity"], "pass", "{v}");
    fs::write(&blob, text).unwrap();
    fs::remove_file(&blob).unwrap();
    let v = verify(d.path(), &first);
    assert_eq!(check(&v, "checkpoints_integrity").unwrap()["result"], "partial", "a missing events blob is partial, not a pass: {v}");
}

#[test]
fn without_an_audit_commit_there_is_nothing_to_attach_to_so_events_wait_for_the_first_audit() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init", "--no-hooks"]).0, 0);
    prompt(d.path(), "first question");
    assert_eq!(stop(d.path(), false).0, 0);
    assert!(!d.path().join(".miaudit/checkpoints").exists(), "no checkpoint without an audit commit");
    let pending = audit(d.path(), &["events"]).1["result"]["events_since_last_audit_commit"].as_u64().unwrap();
    assert!(pending >= 2, "the events stay pending: {pending}");
    fs::write(d.path().join("a.txt"), "x").unwrap();
    assert_eq!(audit(d.path(), &["add", "a.txt"]).0, 0);
    let (code, v) = audit(d.path(), &["commit", "-m", "first audit"]);
    assert_eq!(code, 0, "{v}");
    let m = manifest(d.path(), v["result"]["audit_id"].as_str().unwrap());
    let caps: Vec<(&str, &str)> = m["coverage"].as_array().unwrap().iter().map(|c| (c["capability"].as_str().unwrap(), c["provenance"].as_str().unwrap())).collect();
    assert!(caps.contains(&("conversation_capture", "observed")), "the first audit carries the earlier conversation: {caps:?}");
}

#[test]
fn unaudited_edits_still_block_and_are_never_swallowed_by_a_checkpoint() {
    let (d, first) = repo_with_first_audit();
    fs::write(d.path().join("b.txt"), "two").unwrap();
    edit(d.path(), "b.txt");
    prompt(d.path(), "do it");
    let (code, err) = stop(d.path(), false);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("edited since the last audit commit"), "{err}");
    assert!(checkpoints(d.path(), &first).is_empty());
    let (code, _) = stop(d.path(), true);
    assert_eq!(code, 0, "the second Stop lets the agent finish");
    assert!(checkpoints(d.path(), &first).is_empty(), "and still no checkpoint, so the unaudited edit stays visible");
    assert!(audit(d.path(), &["events"]).1["result"]["events_since_last_audit_commit"].as_u64().unwrap() > 0);
}

#[test]
fn staged_work_and_pending_records_are_left_for_the_explicit_audit() {
    let (d, first) = repo_with_first_audit();
    fs::write(d.path().join("c.txt"), "three").unwrap();
    assert_eq!(audit(d.path(), &["add", "c.txt"]).0, 0);
    prompt(d.path(), "p");
    assert_eq!(stop(d.path(), false).0, 0);
    assert!(checkpoints(d.path(), &first).is_empty(), "the events belong to the audit that is about to be committed");
    assert_eq!(audit(d.path(), &["status"]).1["result"]["staged_changes"].as_array().unwrap().len(), 1);

    let (d2, first2) = repo_with_first_audit();
    fs::write(d2.path().join("ctx.json"), r#"{"schema_version":"1","description":"pending"}"#).unwrap();
    assert_eq!(audit(d2.path(), &["context", "add", "--file", "ctx.json"]).0, 0);
    prompt(d2.path(), "p");
    assert_eq!(stop(d2.path(), false).0, 0);
    assert!(checkpoints(d2.path(), &first2).is_empty());
    assert_eq!(audit(d2.path(), &["status"]).1["result"]["pending"]["contexts"], 1);
}

fn stop_without_payload(d: &Path) -> i32 {
    let mut c = Command::new(BIN).current_dir(d).arg("stop-check").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    drop(c.stdin.take());
    c.wait().unwrap().code().unwrap()
}

#[test]
fn checkpoints_can_be_turned_off_and_on_and_do_nothing_without_events() {
    let (d, first) = repo_with_first_audit();
    assert_eq!(stop_without_payload(d.path()), 0);
    assert!(checkpoints(d.path(), &first).is_empty(), "no events, no checkpoint");

    let status = audit(d.path(), &["checkpoints"]).1;
    assert_eq!((status["result"]["checkpoints"].as_bool(), status["result"]["stored_setting"].as_str()), (Some(true), Some("on (default)")));
    assert_eq!(audit(d.path(), &["checkpoints", "off"]).1["result"]["checkpoints"], false);
    assert!(fs::read_to_string(d.path().join(".miaudit/config.json")).unwrap().contains("\"checkpoints\": false"));
    prompt(d.path(), "p");
    assert_eq!(stop(d.path(), false).0, 0);
    assert!(checkpoints(d.path(), &first).is_empty(), "off means off");
    assert_eq!(audit(d.path(), &["checkpoints", "on"]).1["result"]["checkpoints"], true);
    assert!(!fs::read_to_string(d.path().join(".miaudit/config.json")).unwrap().contains("checkpoints"), "back to the default, nothing stored");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_eq!(checkpoints(d.path(), &first).len(), 1);
    assert_eq!(audit(d.path(), &["checkpoints", "maybe"]).1["error"]["code"], "BAD_ARGUMENT");

    let (d2, first2) = repo_with_first_audit();
    prompt(d2.path(), "p");
    let (code, _) = hook_env(d2.path(), &[("MIAUDIT_CHECKPOINT", "0")], "stop-check", &json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": d2.path(), "stop_hook_active": false}));
    assert_eq!(code, 0);
    assert!(checkpoints(d2.path(), &first2).is_empty(), "MIAUDIT_CHECKPOINT=0 disables it for one run");
}

#[test]
fn a_broken_event_log_blocks_and_is_not_checkpointed() {
    let (d, first) = repo_with_first_audit();
    prompt(d.path(), "p");
    let log_path = d.path().join(".miaudit/events.jsonl");
    let text = fs::read_to_string(&log_path).unwrap().replacen("\"seq\":1", "\"seq\":7", 1);
    fs::write(&log_path, text).unwrap();
    let (code, err) = stop(d.path(), false);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("integrity check"), "{err}");
    assert!(checkpoints(d.path(), &first).is_empty());
}

#[test]
fn a_new_audit_becomes_the_target_of_later_checkpoints_and_says_where_the_earlier_events_went() {
    let (d, first) = repo_with_first_audit();
    prompt(d.path(), "p");
    bash(d.path(), "ls");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_eq!(checkpoints(d.path(), &first).len(), 1);

    fs::write(d.path().join("ctx.json"), r#"{"schema_version":"1","description":"what the last turn meant"}"#).unwrap();
    assert_eq!(audit(d.path(), &["context", "add", "--file", "ctx.json"]).0, 0);
    let (code, v) = audit(d.path(), &["commit", "-m", "explain the last turn"]);
    assert_eq!(code, 0, "{v}");
    let second = v["result"]["audit_id"].as_str().unwrap().to_string();
    let m = manifest(d.path(), &second);
    assert_eq!(m["parents"][0], first.as_str());
    let gap = m["gaps"].as_array().unwrap().iter().find(|g| g["code"] == "CAPTURE_UNAVAILABLE:command_execution_receipts").expect("no events since the checkpoint");
    let text = gap["message"].as_str().unwrap();
    assert!(text.contains("attached to that audit commit as checkpoints"), "{text}");
    assert!(checkpoints(d.path(), &second).is_empty());

    prompt(d.path(), "next");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_eq!(checkpoints(d.path(), &second).len(), 1, "later turns attach to the newest audit");
    assert_eq!(checkpoints(d.path(), &second)[0]["body"]["previous"], Value::Null);
    assert_eq!(checkpoints(d.path(), &first).len(), 1, "the earlier audit keeps what it had");
    assert_eq!(verify(d.path(), &second)["verified"]["integrity"], "pass");
    assert_eq!(verify(d.path(), &first)["verified"]["integrity"], "pass");
}

#[test]
fn a_follow_up_audit_without_file_changes_and_a_superseding_decision() {
    let (d, first) = repo_with_first_audit();
    let snap = audit(d.path(), &["status"]).1["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    let old = |extra: &str| format!(r#"{{"schema_version":"1","question":"keys?","choice":"left for the developer","alternatives":["add them"],"rationale":"needs login","risks":[],"snapshot_id":"{snap}"{extra}}}"#);
    fs::write(d.path().join("old.json"), old("")).unwrap();
    let (_, v) = audit(d.path(), &["decision", "add", "--file", "old.json"]);
    let old_id = v["result"]["id"].as_str().unwrap().to_string();
    let (code, v) = audit(d.path(), &["commit", "-m", "decision about the keys"]);
    assert_eq!(code, 0, "{v}");

    let snap2 = audit(d.path(), &["status"]).1["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    let corrected = format!(r#"{{"schema_version":"1","question":"keys?","choice":"added by the agent with the developer's token","alternatives":[],"rationale":"the developer provided a token","risks":[],"snapshot_id":"{snap2}","supersedes":["{old_id}"]}}"#);
    fs::write(d.path().join("new.json"), corrected).unwrap();
    let (code, v) = audit(d.path(), &["decision", "add", "--file", "new.json"]);
    assert_eq!(code, 0, "{v}");
    fs::write(d.path().join("ev.json"), r#"{"schema_version":"1","classification":"internal","description":"rollout output","content":"registry.dev1 does not resolve"}"#).unwrap();
    assert_eq!(audit(d.path(), &["evidence", "add", "--file", "ev.json"]).0, 0);
    let (code, v) = audit(d.path(), &["commit", "-m", "follow-up: keys added, rollout failed"]);
    assert_eq!(code, 0, "an audit with no file changes is accepted once records are pending: {v}");
    let id = v["result"]["audit_id"].as_str().unwrap();
    let m = manifest(d.path(), id);
    assert_eq!(m["snapshot"]["changes"].as_array().unwrap().len(), 0);
    assert_eq!(m["parents"][0].as_str(), Some(log(d.path())[1]["audit_id"].as_str().unwrap()));
    assert_ne!(first, id);
    let shown = audit(d.path(), &["show", id]).1;
    let decision = &shown["result"]["objects"]["decisions"][0]["object"]["body"];
    assert_eq!(decision["supersedes"][0], old_id.as_str());
    assert_eq!(audit(d.path(), &["verify", id]).1["result"]["verified"]["integrity"], "pass");

    fs::write(d.path().join("bad.json"), format!(r#"{{"schema_version":"1","question":"q","choice":"c","alternatives":[],"rationale":"r","risks":[],"snapshot_id":"{snap2}","supersedes":["dec_{}"]}}"#, "0".repeat(64))).unwrap();
    let (code, v) = audit(d.path(), &["decision", "add", "--file", "bad.json"]);
    assert_eq!((code, v["error"]["code"].as_str(), v["error"]["field"].as_str()), (2, Some("DECISION_REFERENCE_UNAVAILABLE"), Some("/supersedes/0")));
}
