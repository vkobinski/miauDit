
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn hook(dir: &Path, sub: &str, input: &[u8]) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_miaudit"))
        .current_dir(dir)
        .arg(sub)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

fn repo() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init"]).0, 0);
    d
}

fn captured(d: &Path, v: Value) {
    let (code, out, _) = hook(d, "capture", v.to_string().as_bytes());
    assert_eq!(code, 0);
    assert_eq!(out, "", "capture must never print to stdout (it would enter the model context)");
}

fn edit_event(d: &Path, rel: &str, tool: &str) {
    captured(d, json!({"hook_event_name": "PostToolUse", "session_id": "s1", "cwd": d, "tool_name": tool,
        "tool_input": {"file_path": d.join(rel), "content": "SHOULD-NOT-BE-STORED"}, "tool_response": {}}));
}

fn gap_codes(d: &Path) -> Vec<String> {
    audit(d, &["gaps"]).1["result"]["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap().to_string()).collect()
}

fn subjects(d: &Path, code: &str) -> Vec<String> {
    let v = audit(d, &["gaps"]).1;
    v["result"]["gaps"].as_array().unwrap().iter().find(|g| g["code"] == code)
        .map(|g| g["subjects"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn log_text(d: &Path) -> String {
    fs::read_to_string(d.join(".audit/events.jsonl")).unwrap()
}

#[test]
fn capture_records_prompt_edit_and_command_without_content() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "hello").unwrap();
    captured(p, json!({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "cwd": p, "prompt": "please add a.txt"}));
    edit_event(p, "a.txt", "Write");
    captured(p, json!({"hook_event_name": "PostToolUse", "session_id": "s1", "cwd": p, "tool_name": "Bash",
        "tool_input": {"command": "API_TOKEN=hunter2 ~/bin/curl https://x.test --header 'Authorization: abc'"}, "tool_response": {"interrupted": false}}));

    let text = log_text(p);
    for secret in ["hunter2", "SHOULD-NOT-BE-STORED", "Authorization", "please add a.txt"] {
        assert!(!text.contains(secret), "events log must not contain {secret:?}");
    }
    let ev = audit(p, &["events"]).1;
    assert_eq!(ev["result"]["chain"], "ok");
    assert_eq!(ev["result"]["events_total"], 3);
    let recent = ev["result"]["recent"].as_array().unwrap();
    assert_eq!(recent[0]["kind"], "user_prompt");
    assert_eq!(recent[1]["data"]["path"], "a.txt");
    assert!(recent[1]["data"]["digest_after"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(recent[2]["data"]["program"], "curl");
    let blob = recent[0]["data"]["prompt_blob"].as_str().unwrap().trim_start_matches("sha256:").to_string();
    assert_eq!(fs::read_to_string(p.join(".audit/blobs").join(blob)).unwrap(), "please add a.txt");
}

#[test]
fn capture_never_blocks_and_records_unreadable_input() {
    let d = repo();
    let (code, out, _) = hook(d.path(), "capture", b"{not json");
    assert_eq!((code, out.as_str()), (0, ""));
    let ev = audit(d.path(), &["events"]).1;
    assert_eq!(ev["result"]["recent"][0]["kind"], "capture_error");
    assert!(gap_codes(d.path()).contains(&"CAPTURE_ERRORS".to_string()));
    let bare = tempfile::tempdir().unwrap();
    let (code, out, _) = hook(bare.path(), "capture", b"{}");
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn tampering_with_the_event_log_is_detected() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "x").unwrap();
    for _ in 0..3 {
        edit_event(p, "a.txt", "Write");
    }
    let path = p.join(".audit/events.jsonl");
    let lines: Vec<String> = fs::read_to_string(&path).unwrap().lines().map(String::from).collect();
    fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();
    let (code, v) = audit(p, &["events"]);
    assert_eq!(code, 2);
    assert_eq!(v["result"]["chain"], "broken");
    assert!(gap_codes(p).contains(&"EVENT_CHAIN_BROKEN".to_string()));
}

#[test]
fn gaps_cross_check_observed_edits_against_the_snapshot() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("seen.txt"), "1").unwrap();
    fs::write(p.join("unseen.txt"), "2").unwrap();
    fs::write(p.join("later.txt"), "3").unwrap();
    fs::write(p.join("forgotten.txt"), "4").unwrap();
    assert_eq!(audit(p, &["add", "seen.txt", "unseen.txt", "later.txt"]).0, 0);
    assert!(gap_codes(p).contains(&"NO_OBSERVED_EVENTS".to_string()));

    edit_event(p, "seen.txt", "Write");
    edit_event(p, "later.txt", "Edit");
    edit_event(p, "forgotten.txt", "Write");
    fs::write(p.join("later.txt"), "3 changed by a command").unwrap();
    assert_eq!(audit(p, &["add", "later.txt"]).0, 0);

    assert!(!gap_codes(p).contains(&"NO_OBSERVED_EVENTS".to_string()));
    assert_eq!(subjects(p, "STAGED_WITHOUT_OBSERVED_EDIT"), vec!["unseen.txt"]);
    assert_eq!(subjects(p, "STAGED_DIFFERS_FROM_OBSERVED"), vec!["later.txt"]);
    assert_eq!(subjects(p, "OBSERVED_EDIT_NOT_STAGED"), vec!["forgotten.txt"]);
}

#[test]
fn editing_the_audit_store_with_a_tool_is_flagged() {
    let d = repo();
    let p = d.path();
    captured(p, json!({"hook_event_name": "PostToolUse", "cwd": p, "tool_name": "Edit",
        "tool_input": {"file_path": p.join(".audit/HEAD")}, "tool_response": {}}));
    assert_eq!(subjects(p, "AUDIT_STORE_TOUCHED_BY_TOOL").len(), 1);
}

#[test]
fn commit_attaches_observed_events_and_verify_checks_the_chain() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "hello").unwrap();
    captured(p, json!({"hook_event_name": "UserPromptSubmit", "cwd": p, "prompt": "do it"}));
    edit_event(p, "a.txt", "Write");
    assert_eq!(audit(p, &["add", "a.txt"]).0, 0);
    let (code, c) = audit(p, &["commit", "-m", "with hooks"]);
    assert_eq!(code, 0, "{c}");
    let id = c["result"]["audit_id"].as_str().unwrap().to_string();

    let shown = audit(p, &["show", &id]).1;
    let m = &shown["result"]["manifest"];
    let caps: Vec<(&str, &str)> = m["coverage"].as_array().unwrap().iter().map(|c| (c["capability"].as_str().unwrap(), c["provenance"].as_str().unwrap())).collect();
    assert!(caps.contains(&("hook_file_edits", "observed")));
    assert!(caps.contains(&("hook_user_prompts", "observed")));
    assert!(!caps.iter().any(|(n, _)| *n == "hook_command_digests"), "no commands were observed");
    let evid = shown["result"]["objects"]["evidence"].as_array().unwrap();
    assert_eq!(evid.len(), 1);
    assert_eq!(evid[0]["object"]["provenance"], "observed");

    let (vcode, v) = audit(p, &["verify", &id]);
    assert_eq!(vcode, 0, "{v}");
    let checks = v["result"]["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["check"] == "observed_events_chain" && c["result"] == "pass"));

    fs::write(p.join("b.txt"), "b").unwrap();
    assert_eq!(audit(p, &["add", "b.txt"]).0, 0);
    let (_, c2) = audit(p, &["commit", "-m", "no hooks this time"]);
    let id2 = c2["result"]["audit_id"].as_str().unwrap();
    let shown2 = audit(p, &["show", id2]).1;
    assert_eq!(shown2["result"]["objects"]["evidence"].as_array().unwrap().len(), 0);
    assert_eq!(audit(p, &["events"]).1["result"]["events_since_last_audit_commit"], 0);
}

#[test]
fn altering_an_attached_event_blob_fails_verification() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "hello").unwrap();
    edit_event(p, "a.txt", "Write");
    edit_event(p, "a.txt", "Write");
    audit(p, &["add", "a.txt"]);
    let id = audit(p, &["commit", "-m", "m"]).1["result"]["audit_id"].as_str().unwrap().to_string();
    let shown = audit(p, &["show", &id]).1;
    let digest = {
        let evd = shown["result"]["objects"]["evidence"][0]["object"]["body"]["artifact"]["digest"].as_str().unwrap().to_string();
        evd.trim_start_matches("sha256:").to_string()
    };
    let blob = p.join(".audit/blobs").join(digest);
    let mut text = fs::read_to_string(&blob).unwrap();
    text = text.replacen("\"a.txt\"", "\"z.txt\"", 1);
    fs::write(&blob, text).unwrap();
    let (code, v) = audit(p, &["verify", &id]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn stop_check_blocks_once_on_unaudited_edits() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "x").unwrap();
    assert_eq!(hook(p, "stop-check", br#"{"hook_event_name":"Stop"}"#).0, 0);
    captured(p, json!({"hook_event_name": "UserPromptSubmit", "cwd": p, "prompt": "chat only"}));
    assert_eq!(hook(p, "stop-check", br#"{"hook_event_name":"Stop"}"#).0, 0, "prompts alone are not unaudited work");

    edit_event(p, "a.txt", "Write");
    let (code, out, err) = hook(p, "stop-check", br#"{"hook_event_name":"Stop"}"#);
    assert_eq!(code, 2);
    assert_eq!(out, "");
    assert!(err.contains("a.txt") && err.contains("audit"), "{err}");
    assert_eq!(hook(p, "stop-check", br#"{"hook_event_name":"Stop","stop_hook_active":true}"#).0, 0);

    audit(p, &["add", "a.txt"]);
    assert_eq!(audit(p, &["commit", "-m", "m"]).0, 0);
    assert_eq!(hook(p, "stop-check", br#"{"hook_event_name":"Stop"}"#).0, 0);
}

#[test]
fn parallel_captures_keep_the_chain_intact() {
    let d = repo();
    let p = d.path().to_path_buf();
    let handles: Vec<_> = (0..8).map(|i| {
        let p = p.clone();
        std::thread::spawn(move || captured(&p, json!({"hook_event_name": "PostToolUse", "cwd": p, "tool_name": "Bash", "tool_input": {"command": format!("echo {i}")}, "tool_response": {}})))
    }).collect();
    for h in handles {
        h.join().unwrap();
    }
    let v = audit(&p, &["events"]).1;
    assert_eq!(v["result"]["chain"], "ok");
    assert_eq!(v["result"]["events_total"], 8);
}
