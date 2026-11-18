use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
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
    hook_env(d, &[("MIAUDIT_GITHUB_USER", "test-user")], "stop-check", &json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": d, "stop_hook_active": again, "last_assistant_message": "done"}))
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

fn manifest(d: &Path, id: &str) -> Value {
    audit(d, &["show", id]).1["result"]["manifest"].clone()
}

fn codes(m: &Value) -> Vec<String> {
    m["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_turn_with_only_commands_and_conversation_gets_an_observed_only_checkpoint() {
    let (d, first) = repo_with_first_audit();
    prompt(d.path(), "rotate the keys");
    bash(d.path(), "bao kv put secret/x a=1");
    let (code, err) = stop(d.path(), false);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("checkpoint audit_"), "{err}");

    let commits = log(d.path());
    assert_eq!(commits.len(), 2, "one new audit commit");
    let id = head(d.path());
    let m = manifest(d.path(), &id);
    assert!(m["description"].as_str().unwrap().starts_with("checkpoint: 1 prompt, 1 assistant message digest, 1 command"), "{}", m["description"]);
    assert_eq!(m["parents"][0], first.as_str());
    assert_eq!(m["snapshot"]["changes"].as_array().unwrap().len(), 0, "a checkpoint carries no file changes");
    assert!(m["context_ids"].as_array().unwrap().is_empty() && m["decision_ids"].as_array().unwrap().is_empty());
    let c = codes(&m);
    assert!(c.contains(&"AUTO_CHECKPOINT".to_string()) && c.contains(&"NO_CONTEXT".to_string()) && c.contains(&"NO_DECISION".to_string()), "{c:?}");
    let caps: Vec<(&str, &str)> = m["coverage"].as_array().unwrap().iter().map(|c| (c["capability"].as_str().unwrap(), c["provenance"].as_str().unwrap())).collect();
    assert!(caps.contains(&("conversation_capture", "observed")) && caps.contains(&("command_execution_receipts", "observed")), "{caps:?}");
    assert_eq!(m["evidence_ids"].as_array().unwrap().len(), 1, "the observed event receipts are the only evidence");
    assert_eq!(m["author"]["provenance"], "self_reported", "MIAUDIT_GITHUB_USER is honored without a network call");

    let events = audit(d.path(), &["events"]).1;
    assert_eq!(events["result"]["events_since_last_audit_commit"], 0, "the cursor advanced");
    assert_eq!(audit(d.path(), &["verify", &id]).1["result"]["verified"]["integrity"], "pass");

    let stored = fs::read_dir(d.path().join(".miaudit/blobs")).unwrap().map(|e| fs::read(e.unwrap().path()).unwrap()).map(|b| String::from_utf8_lossy(&b).to_string()).collect::<String>();
    assert!(!stored.contains("SECRET-OUTPUT") && !stored.contains("bao kv put"), "only digests are kept for commands");
}

#[test]
fn the_checkpoint_never_calls_gh() {
    let (d, _) = repo_with_first_audit();
    let bin = tempfile::tempdir().unwrap();
    let marker = bin.path().join("gh-was-called");
    let script = bin.path().join("gh");
    fs::write(&script, format!("#!/bin/sh\ntouch {}\necho '{{\"login\":\"octo\",\"id\":1}}'\n", marker.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.path().display());
    let payload = json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": d.path(), "stop_hook_active": false, "last_assistant_message": "x"});
    let mut cmd = Command::new(BIN);
    cmd.current_dir(d.path()).arg("stop-check").env("PATH", path).env_remove("MIAUDIT_GITHUB_USER").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut c = cmd.spawn().unwrap();
    c.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    assert_eq!(c.wait().unwrap().code(), Some(0));
    assert!(!marker.exists(), "gh must not run inside the Stop hook");
    let m = manifest(d.path(), &head(d.path()));
    assert!(codes(&m).contains(&"AUTHOR_UNAVAILABLE".to_string()));
    assert!(m["author"]["verification"].as_str().unwrap().contains("gh lookup is skipped"));
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
    assert_eq!(head(d.path()), first, "no checkpoint while edits wait for a real audit");
    let (code, _) = stop(d.path(), true);
    assert_eq!(code, 0, "the second Stop lets the agent finish");
    assert_eq!(head(d.path()), first, "and still no checkpoint, so the unaudited edit stays visible");
    assert!(audit(d.path(), &["events"]).1["result"]["events_since_last_audit_commit"].as_u64().unwrap() > 0);
}

#[test]
fn staged_work_and_pending_records_are_left_for_the_explicit_audit() {
    let (d, first) = repo_with_first_audit();
    fs::write(d.path().join("c.txt"), "three").unwrap();
    assert_eq!(audit(d.path(), &["add", "c.txt"]).0, 0);
    prompt(d.path(), "p");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_eq!(head(d.path()), first, "staged changes are not committed behind the agent's back");
    assert_eq!(audit(d.path(), &["status"]).1["result"]["staged_changes"].as_array().unwrap().len(), 1);

    let (d2, first2) = repo_with_first_audit();
    fs::write(d2.path().join("ctx.json"), r#"{"schema_version":"1","description":"pending"}"#).unwrap();
    assert_eq!(audit(d2.path(), &["context", "add", "--file", "ctx.json"]).0, 0);
    prompt(d2.path(), "p");
    assert_eq!(stop(d2.path(), false).0, 0);
    assert_eq!(head(d2.path()), first2, "pending context is left for the explicit audit");
    assert_eq!(audit(d2.path(), &["status"]).1["result"]["pending"]["contexts"], 1);
}

#[test]
fn checkpoints_can_be_turned_off_and_on_and_do_nothing_without_events() {
    let (d, first) = repo_with_first_audit();
    assert_eq!(stop_without_payload(d.path()), 0);
    assert_eq!(head(d.path()), first, "no events, no checkpoint");

    let status = audit(d.path(), &["checkpoints"]).1;
    assert_eq!((status["result"]["checkpoints"].as_bool(), status["result"]["stored_setting"].as_str()), (Some(true), Some("on (default)")));
    assert_eq!(audit(d.path(), &["checkpoints", "off"]).1["result"]["checkpoints"], false);
    assert!(fs::read_to_string(d.path().join(".miaudit/config.json")).unwrap().contains("\"checkpoints\": false"));
    prompt(d.path(), "p");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_eq!(head(d.path()), first, "off means off");
    assert_eq!(audit(d.path(), &["checkpoints", "on"]).1["result"]["checkpoints"], true);
    assert!(!fs::read_to_string(d.path().join(".miaudit/config.json")).unwrap().contains("checkpoints"), "back to the default, nothing stored");
    assert_eq!(stop(d.path(), false).0, 0);
    assert_ne!(head(d.path()), first);
    assert_eq!(audit(d.path(), &["checkpoints", "maybe"]).1["error"]["code"], "BAD_ARGUMENT");

    let (d2, first2) = repo_with_first_audit();
    prompt(d2.path(), "p");
    let (code, _) = hook_env(d2.path(), &[("MIAUDIT_CHECKPOINT", "0")], "stop-check", &json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": d2.path(), "stop_hook_active": false}));
    assert_eq!(code, 0);
    assert_eq!(head(d2.path()), first2, "MIAUDIT_CHECKPOINT=0 disables it for one run");
}

fn stop_without_payload(d: &Path) -> i32 {
    let mut c = Command::new(BIN).current_dir(d).arg("stop-check").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    drop(c.stdin.take());
    c.wait().unwrap().code().unwrap()
}

#[test]
fn every_turn_extends_the_chain_and_each_checkpoint_verifies() {
    let (d, first) = repo_with_first_audit();
    let mut previous = first;
    for turn in 0..3 {
        prompt(d.path(), &format!("question {turn}"));
        bash(d.path(), &format!("kubectl get pods {turn}"));
        assert_eq!(stop(d.path(), false).0, 0);
        let id = head(d.path());
        assert_ne!(id, previous);
        assert_eq!(manifest(d.path(), &id)["parents"][0], previous.as_str());
        assert_eq!(audit(d.path(), &["verify", &id]).1["result"]["verified"]["integrity"], "pass", "turn {turn}");
        previous = id;
    }
    assert_eq!(log(d.path()).len(), 4);
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
    assert_eq!(head(d.path()), first);
}

#[test]
fn the_audit_after_a_checkpoint_says_where_the_earlier_events_went() {
    let (d, _) = repo_with_first_audit();
    prompt(d.path(), "p");
    bash(d.path(), "ls");
    assert_eq!(stop(d.path(), false).0, 0);
    let checkpoint = head(d.path());
    fs::write(d.path().join("ctx.json"), r#"{"schema_version":"1","description":"what the last turn meant"}"#).unwrap();
    assert_eq!(audit(d.path(), &["context", "add", "--file", "ctx.json"]).0, 0);
    let (code, v) = audit(d.path(), &["commit", "-m", "explain the last turn"]);
    assert_eq!(code, 0, "{v}");
    let m = manifest(d.path(), v["result"]["audit_id"].as_str().unwrap());
    assert_eq!(m["parents"][0], checkpoint.as_str());
    let gap = m["gaps"].as_array().unwrap().iter().find(|g| g["code"] == "CAPTURE_UNAVAILABLE:command_execution_receipts").expect("no events since the checkpoint");
    let text = gap["message"].as_str().unwrap();
    assert!(text.contains("automatic checkpoint") && text.contains("preceding turns"), "{text}");
    assert!(!codes(&m).contains(&"AUTO_CHECKPOINT".to_string()), "an explicit audit is not marked as a checkpoint");
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
