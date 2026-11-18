use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn hook(dir: &Path, args: &[&str], input: &Value) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_miaudit")).current_dir(dir).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

fn repo() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init"]).0, 0);
    d
}

fn post(d: &Path, tool: &str, input: Value, response: Value) {
    let (code, out, _) = hook(d, &["capture"], &json!({"hook_event_name": "PostToolUse", "session_id": "s1", "cwd": d, "tool_name": tool, "tool_input": input, "tool_response": response}));
    assert_eq!((code, out.as_str()), (0, ""));
}

fn events(d: &Path) -> Vec<Value> {
    audit(d, &["events"]).1["result"]["recent"].as_array().unwrap().clone()
}

fn log_text(d: &Path) -> String {
    fs::read_to_string(d.join(".miaudit/events.jsonl")).unwrap()
}

#[test]
fn command_receipts_carry_output_digest_and_exit_code_but_no_output_text() {
    let d = repo();
    let p = d.path();
    post(p, "Bash", json!({"command": "cargo test"}), json!({"stdout": "OUTPUT-TEXT-1", "stderr": "ERR-TEXT", "exit_code": 3}));
    post(p, "Bash", json!({"command": "cargo test"}), json!({"stdout": "OUTPUT-TEXT-2", "stderr": "ERR-TEXT", "exit_code": 0}));
    post(p, "Bash", json!({"command": "ls"}), json!("plain string output"));
    post(p, "Bash", json!({"command": "ls"}), json!({"interrupted": true}));
    let ev = events(p);
    assert_eq!(ev[0]["data"]["exit_code"], 3);
    assert_eq!(ev[1]["data"]["exit_code"], 0);
    assert!(ev[0]["data"]["output_digest"].as_str().unwrap().starts_with("sha256:"));
    assert_ne!(ev[0]["data"]["output_digest"], ev[1]["data"]["output_digest"], "different output must give a different digest");
    assert_eq!(ev[0]["data"]["output_bytes"], 13 + 8);
    assert!(ev[2]["data"]["output_digest"].as_str().unwrap().starts_with("sha256:"));
    assert!(ev[2]["data"]["exit_code"].is_null());
    assert!(ev[3]["data"]["output_digest"].is_null());
    let text = log_text(p);
    for secret in ["OUTPUT-TEXT", "ERR-TEXT", "plain string output"] {
        assert!(!text.contains(secret), "{secret} must not be stored");
    }
}

#[test]
fn resource_receipts_keep_paths_hosts_and_digests_only() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "file body SECRET-FILE-BODY").unwrap();
    post(p, "Read", json!({"file_path": p.join("a.txt")}), json!({"content": "file body SECRET-FILE-BODY"}));
    post(p, "Read", json!({"file_path": "/etc/hosts"}), json!({}));
    post(p, "Grep", json!({"pattern": "api_key_regex_SECRET", "path": "src"}), json!({}));
    post(p, "WebFetch", json!({"url": "https://user:pw@docs.example.com:8443/path/x?token=URLTOKEN#frag", "prompt": "PROMPT-TEXT"}), json!({}));
    post(p, "WebSearch", json!({"query": "how to SEARCH-SECRET"}), json!({}));
    post(p, "mcp__slack__send", json!({"channel": "MCP-ARG-SECRET"}), json!({}));
    let ev = events(p);
    assert!(ev.iter().all(|e| e["kind"] == "resource_access"), "{ev:?}");
    assert_eq!(ev[0]["data"]["path"], "a.txt");
    assert!(ev[0]["data"]["digest_after"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(ev[1]["data"]["outside_repo_or_unsupported"], true);
    assert_eq!(ev[3]["data"]["host"], "docs.example.com");
    assert_eq!(ev[4]["data"]["resource"], "web_search");
    assert_eq!(ev[5]["data"]["resource"], "mcp");
    let text = log_text(p);
    for secret in ["SECRET-FILE-BODY", "api_key_regex_SECRET", "URLTOKEN", "user:pw", "/path/x", "PROMPT-TEXT", "SEARCH-SECRET", "MCP-ARG-SECRET"] {
        assert!(!text.contains(secret), "{secret} must not be stored");
    }
}

#[test]
fn agent_topology_records_spawns_and_subagent_lifecycle_without_prompts() {
    let d = repo();
    let p = d.path();
    post(p, "Task", json!({"subagent_type": "reviewer", "prompt": "PROMPT-SECRET please review", "description": "d"}), json!({}));
    post(p, "spawn_agent", json!({"agent_type": "worker", "message": "MSG-SECRET"}), json!({}));
    hook(p, &["capture"], &json!({"hook_event_name": "SubagentStart", "session_id": "s1", "agent_id": "ag-1", "agent_type": "reviewer", "turn_id": "t1"}));
    hook(p, &["capture"], &json!({"hook_event_name": "SubagentStop", "session_id": "s1", "agent_id": "ag-1", "agent_type": "reviewer", "last_assistant_message": "REPLY-SECRET"}));
    let ev = events(p);
    let actions: Vec<&str> = ev.iter().map(|e| e["data"]["action"].as_str().unwrap()).collect();
    assert_eq!(actions, vec!["spawn", "spawn", "start", "stop"]);
    assert_eq!(ev[0]["data"]["agent_type"], "reviewer");
    assert_eq!(ev[2]["data"]["agent_id"], "ag-1");
    assert!(ev[3]["data"]["message_digest"].as_str().unwrap().starts_with("sha256:"));
    let text = log_text(p);
    for secret in ["PROMPT-SECRET", "MSG-SECRET", "REPLY-SECRET"] {
        assert!(!text.contains(secret), "{secret} must not be stored");
    }
}

#[test]
fn stop_records_the_assistant_message_and_transcript_digest_and_still_blocks_on_edits() {
    let d = repo();
    let p = d.path();
    let transcript = tempfile::NamedTempFile::new().unwrap();
    let body = "{\"role\":\"assistant\",\"text\":\"TRANSCRIPT-SECRET\"}\n";
    fs::write(transcript.path(), body).unwrap();
    let input = json!({"hook_event_name": "Stop", "session_id": "s1", "turn_id": "t", "stop_hook_active": false,
        "last_assistant_message": "FINAL-REPLY-SECRET", "transcript_path": transcript.path()});
    let (code, out, _) = hook(p, &["stop-check"], &input);
    assert_eq!((code, out.as_str()), (0, ""));
    let ev = events(p);
    assert_eq!(ev[0]["kind"], "assistant_message");
    assert_eq!(ev[0]["data"]["message_bytes"], 18);
    assert!(ev[0]["data"]["transcript_digest"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(ev[0]["data"]["transcript_bytes"], body.len());
    assert!(!log_text(p).contains("SECRET"));
    assert!(!log_text(p).contains(transcript.path().to_str().unwrap()), "the transcript path must not be stored");

    fs::write(p.join("a.txt"), "x").unwrap();
    post(p, "Write", json!({"file_path": p.join("a.txt")}), json!({}));
    let (code, _, err) = hook(p, &["stop-check"], &input);
    assert_eq!(code, 2);
    assert!(err.contains("a.txt"));
    let codex = json!({"hook_event_name": "Stop", "session_id": "c1", "last_assistant_message": "x", "stop_hook_active": true});
    assert_eq!(hook(p, &["stop-check", "--agent", "codex"], &codex).0, 0);
    assert!(log_text(p).contains("codex_hook"));
}

#[test]
fn all_four_capabilities_become_observed_with_notes_and_gaps_turn_partial() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("a.txt"), "x").unwrap();
    hook(p, &["capture"], &json!({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "cwd": p, "prompt": "do it"}));
    post(p, "Bash", json!({"command": "ls"}), json!({"stdout": "o", "exit_code": 0}));
    post(p, "Read", json!({"file_path": p.join("a.txt")}), json!({}));
    post(p, "Task", json!({"subagent_type": "x", "prompt": "p"}), json!({}));
    assert_eq!(audit(p, &["add", "a.txt"]).0, 0);
    hook(p, &["stop-check"], &json!({"hook_event_name": "Stop", "session_id": "s1", "last_assistant_message": "done"}));
    let (code, c) = audit(p, &["commit", "-m", "m"]);
    assert_eq!(code, 0, "{c}");
    let id = c["result"]["audit_id"].as_str().unwrap().to_string();
    let m = audit(p, &["show", &id]).1["result"]["manifest"].clone();
    for cap in ["conversation_capture", "agent_topology", "resource_access_receipts", "command_execution_receipts"] {
        let row = m["coverage"].as_array().unwrap().iter().find(|c| c["capability"] == cap).unwrap();
        assert_eq!(row["provenance"], "observed", "{cap}");
        assert!(row["note"].as_str().unwrap().starts_with("partial (hook)"), "{cap}: {row}");
        let codes: Vec<&str> = m["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap()).collect();
        assert!(codes.contains(&format!("CAPTURE_PARTIAL:{cap}").as_str()), "{cap}");
        assert!(!codes.contains(&format!("CAPTURE_UNAVAILABLE:{cap}").as_str()), "{cap}");
    }
    assert_eq!(audit(p, &["verify", &id]).0, 0);
}

#[test]
fn shipped_hook_configs_wire_every_event_the_capabilities_depend_on() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks");
    let claude: Value = serde_json::from_str(&fs::read_to_string(dir.join("claude-code.settings.json")).unwrap()).unwrap();
    let codex: Value = serde_json::from_str(&fs::read_to_string(dir.join("codex.hooks.json")).unwrap()).unwrap();
    for (name, cfg) in [("claude", &claude), ("codex", &codex)] {
        for ev in ["SessionStart", "UserPromptSubmit", "PostToolUse", "SubagentStart", "SubagentStop", "Stop"] {
            assert!(cfg["hooks"].get(ev).is_some(), "{name} is missing {ev}");
        }
    }
    let matcher = claude["hooks"]["PostToolUse"][0]["matcher"].as_str().unwrap();
    for tool in ["Bash", "Read", "Grep", "Glob", "WebFetch", "WebSearch", "Task", "mcp__"] {
        assert!(matcher.contains(tool), "claude matcher lacks {tool}");
    }
    let cm = codex["hooks"]["PostToolUse"][0]["matcher"].as_str().unwrap();
    for tool in ["Bash", "apply_patch", "spawn_agent", "mcp__"] {
        assert!(cm.contains(tool), "codex matcher lacks {tool}");
    }
    assert!(codex["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap().contains("--agent codex"));
    let live: Value = serde_json::from_str(&fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.claude/settings.json")).unwrap()).unwrap();
    assert_eq!(live, claude, ".claude/settings.json must match the shipped example");
}
