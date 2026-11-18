use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).env("HOME", dir).current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn settings(dir: &Path, name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join(".claude").join(name)).unwrap()).unwrap()
}

fn template() -> Value {
    serde_json::from_str(include_str!("../hooks/claude-code.settings.json")).unwrap()
}

fn count(v: &Value, event: &str, command: &str) -> usize {
    v["hooks"][event].as_array().unwrap().iter().flat_map(|e| e["hooks"].as_array().unwrap()).filter(|h| h["command"] == command).count()
}

#[test]
fn init_creates_the_store_and_links_every_claude_hook() {
    let d = tempfile::tempdir().unwrap();
    let (code, v) = audit(d.path(), &["init"]);
    assert_eq!(code, 0, "{v}");
    assert!(d.path().join(".miaudit/config.json").is_file());
    let r = &v["result"]["hooks"];
    assert_eq!(r["installed"], true);
    assert_eq!(r["agent"], "claude-code");
    assert_eq!(r["created_file"], true);
    let s = settings(d.path(), "settings.json");
    let t = template();
    let events: Vec<&String> = t["hooks"].as_object().unwrap().keys().collect();
    assert!(events.len() >= 6, "{events:?}");
    for e in &events {
        assert!(s["hooks"][e.as_str()].is_array(), "{e} missing");
    }
    assert_eq!(s, t, "a fresh settings file is exactly the bundled template");
    assert_eq!(r["added"].as_array().unwrap().len(), events.len());
    assert!(r["warning"].as_str().unwrap().contains("does not exist"), "HOME here has no installed binary, so init says so");
    assert!(r["note"].as_str().unwrap().contains("/hooks"));
}

#[test]
fn init_twice_changes_nothing_and_never_duplicates_a_hook() {
    let d = tempfile::tempdir().unwrap();
    audit(d.path(), &["init"]);
    let path = d.path().join(".claude/settings.json");
    let before = fs::read(&path).unwrap();
    let (code, v) = audit(d.path(), &["init"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["already_initialized"], true);
    assert_eq!(v["result"]["hooks"]["added"].as_array().unwrap().len(), 0);
    assert_eq!(v["result"]["hooks"]["created_file"], false);
    assert_eq!(fs::read(&path).unwrap(), before, "no rewrite when nothing is missing");
    let s = settings(d.path(), "settings.json");
    assert_eq!(count(&s, "UserPromptSubmit", "$HOME/.cargo/bin/miaudit capture"), 1);
}

#[test]
fn existing_settings_and_other_hooks_are_kept() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".claude")).unwrap();
    let mine = json!({
        "permissions": {"allow": ["Bash(ls:*)"]},
        "hooks": {
            "Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}],
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "$HOME/.cargo/bin/miaudit capture"}]}]
        }
    });
    fs::write(d.path().join(".claude/settings.json"), serde_json::to_string_pretty(&mine).unwrap()).unwrap();
    let (code, v) = audit(d.path(), &["init"]);
    assert_eq!(code, 0, "{v}");
    let s = settings(d.path(), "settings.json");
    assert_eq!(s["permissions"], mine["permissions"]);
    assert_eq!(count(&s, "Stop", "echo mine"), 1, "an unrelated hook on the same event survives");
    assert_eq!(count(&s, "Stop", "$HOME/.cargo/bin/miaudit stop-check"), 1, "ours is added next to it");
    assert_eq!(count(&s, "UserPromptSubmit", "$HOME/.cargo/bin/miaudit capture"), 1, "an identical hook is not duplicated");
    let added: Vec<&str> = v["result"]["hooks"]["added"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(added.contains(&"Stop") && !added.contains(&"UserPromptSubmit"), "{added:?}");
}

#[test]
fn unreadable_settings_stop_init_before_anything_is_created() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".claude")).unwrap();
    let path = d.path().join(".claude/settings.json");
    for broken in ["{not json", "[1,2]", r#"{"hooks": 5}"#, r#"{"hooks": {"Stop": "x"}}"#] {
        fs::write(&path, broken).unwrap();
        let (code, v) = audit(d.path(), &["init"]);
        assert_eq!((code, v["error"]["code"].as_str()), (2, Some("HOOKS_SETTINGS_INVALID")), "{broken}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken, "the file is left alone");
        assert!(!d.path().join(".miaudit").exists(), "no half-initialized store for {broken}");
    }
}

#[test]
fn no_hooks_and_local_choose_where_or_whether() {
    let d = tempfile::tempdir().unwrap();
    let (code, v) = audit(d.path(), &["init", "--no-hooks"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["hooks"]["installed"], false);
    assert!(!d.path().join(".claude").exists());
    let (code, v) = audit(d.path(), &["init", "--local"]);
    assert_eq!(code, 0, "{v}");
    assert!(d.path().join(".claude/settings.local.json").is_file());
    assert!(!d.path().join(".claude/settings.json").exists());
    assert_eq!(settings(d.path(), "settings.local.json"), template());
}

#[test]
fn init_from_a_subdirectory_links_hooks_at_the_store_root() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init", "--no-hooks"]).0, 0);
    let sub = d.path().join("a/b");
    fs::create_dir_all(&sub).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).env("HOME", d.path()).current_dir(&sub).args(["--json", "init"]).output().unwrap();
    assert!(out.status.success());
    assert!(d.path().join(".claude/settings.json").is_file(), "hooks go next to .miaudit, not into the subdirectory");
    assert!(!sub.join(".claude").exists());
}

#[test]
fn installed_hooks_drive_capture_end_to_end() {
    let d = tempfile::tempdir().unwrap();
    audit(d.path(), &["init"]);
    let s = settings(d.path(), "settings.json");
    let capture = s["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(capture.ends_with("miaudit capture"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_miaudit")).arg("capture").current_dir(d.path()).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(json!({"hook_event_name": "UserPromptSubmit", "cwd": d.path(), "prompt": "hello", "session_id": "s1"}).to_string().as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    let events = audit(d.path(), &["events"]).1;
    assert_eq!(events["result"]["events_total"], 1, "{events}");
    assert_eq!(events["result"]["chain"], "ok");
}
