use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn repo(kind: Option<&str>) -> TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    assert_eq!(audit(p, &["init"]).0, 0);
    fs::write(p.join("f.txt"), "x").unwrap();
    assert_eq!(audit(p, &["add", "f.txt"]).0, 0);
    let mut ctx = json!({"schema_version": "1", "description": "task"});
    if let Some(k) = kind {
        ctx["change_kind"] = json!(k);
    }
    fs::write(p.join("ctx.json"), ctx.to_string()).unwrap();
    assert_eq!(audit(p, &["context", "add", "--file", "ctx.json"]).0, 0);
    d
}

fn add_evidence(p: &Path, extra: Value, file: Option<&Path>) -> (i32, Value) {
    let mut e = json!({"schema_version": "1", "classification": "internal", "description": "evidence"});
    match file {
        Some(f) => e["file"] = json!(f),
        None => e["content"] = json!("HTTP/1.1 200 OK"),
    }
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    fs::write(p.join("e.json"), e.to_string()).unwrap();
    audit(p, &["evidence", "add", "--file", "e.json"])
}

fn codes(p: &Path) -> Vec<String> {
    audit(p, &["gaps"]).1["result"]["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap().to_string()).collect()
}

fn png(p: &Path) -> std::path::PathBuf {
    let path = p.join("shot.png");
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend_from_slice(b"fake pixel data");
    fs::write(&path, b).unwrap();
    path
}

#[test]
fn undeclared_change_kind_is_a_gap_and_other_silences_it() {
    let d = repo(None);
    assert!(codes(d.path()).contains(&"CHANGE_KIND_UNDECLARED".to_string()));
    let d = repo(Some("other"));
    let c = codes(d.path());
    assert!(!c.iter().any(|x| x.starts_with("CHANGE_KIND") || x.starts_with("API_EVIDENCE") || x.starts_with("FRONTEND_EVIDENCE")), "{c:?}");
}

#[test]
fn api_changes_need_before_and_after_runs_in_a_real_environment() {
    let d = repo(Some("api"));
    let p = d.path();
    let c = codes(p);
    assert!(c.contains(&"API_EVIDENCE_MISSING:before".to_string()) && c.contains(&"API_EVIDENCE_MISSING:after".to_string()));
    assert!(add_evidence(p, json!({"phase": "before", "environment": "staging"}), None).0 == 0);
    let c = codes(p);
    assert!(!c.contains(&"API_EVIDENCE_MISSING:before".to_string()));
    assert!(c.contains(&"API_EVIDENCE_MISSING:after".to_string()));
    assert_eq!(add_evidence(p, json!({"phase": "after", "environment": "ci"}), None).0, 0);
    assert!(codes(p).contains(&"API_EVIDENCE_MISSING:after".to_string()), "a CI run alone does not show the code running against the API");
    assert_eq!(add_evidence(p, json!({"phase": "after", "environment": "local", "description": "new version run against local API"}), None).0, 0);
    assert!(!codes(p).iter().any(|x| x.starts_with("API_EVIDENCE")));
}

#[test]
fn frontend_changes_need_before_and_after_screenshots() {
    let d = repo(Some("frontend"));
    let p = d.path();
    let c = codes(p);
    assert!(c.contains(&"FRONTEND_EVIDENCE_MISSING:before".to_string()) && c.contains(&"FRONTEND_EVIDENCE_MISSING:after".to_string()));
    assert_eq!(add_evidence(p, json!({"phase": "before", "environment": "local"}), None).0, 0);
    assert!(codes(p).contains(&"FRONTEND_EVIDENCE_MISSING:before".to_string()), "text is not a screenshot");
    let shot = png(p);
    assert_eq!(add_evidence(p, json!({"phase": "before", "environment": "local"}), Some(&shot)).0, 0);
    let c = codes(p);
    assert!(!c.contains(&"FRONTEND_EVIDENCE_MISSING:before".to_string()));
    assert!(c.contains(&"FRONTEND_EVIDENCE_MISSING:after".to_string()));
    assert_eq!(add_evidence(p, json!({"phase": "after", "environment": "staging"}), Some(&shot)).0, 0);
    assert!(!codes(p).iter().any(|x| x.starts_with("FRONTEND_EVIDENCE")));
}

#[test]
fn an_image_extension_does_not_make_text_a_screenshot() {
    let d = repo(Some("frontend"));
    let p = d.path();
    let fake = p.join("fake.png");
    fs::write(&fake, "not an image").unwrap();
    assert_eq!(add_evidence(p, json!({"phase": "before", "environment": "local"}), Some(&fake)).0, 0);
    assert!(codes(p).contains(&"FRONTEND_EVIDENCE_MISSING:before".to_string()));
}

#[test]
fn invalid_phase_and_environment_are_rejected_and_metadata_is_stored() {
    let d = repo(Some("api"));
    let p = d.path();
    assert_eq!(add_evidence(p, json!({"phase": "during"}), None).0, 2);
    assert_eq!(add_evidence(p, json!({"environment": "laptop"}), None).0, 2);
    let (code, v) = add_evidence(p, json!({"phase": "after", "environment": "production"}), None);
    assert_eq!(code, 0);
    let id = v["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(audit(p, &["commit", "-m", "m"]).0, 0);
    let aid = audit(p, &["log"]).1["result"]["commits"][0]["audit_id"].as_str().unwrap().to_string();
    let shown = audit(p, &["show", &aid]).1;
    let ev = shown["result"]["objects"]["evidence"].as_array().unwrap().iter().find(|e| e["id"] == id.as_str()).unwrap();
    assert_eq!(ev["object"]["body"]["phase"], "after");
    assert_eq!(ev["object"]["body"]["environment"], "production");
    assert_eq!(audit(p, &["verify", &aid]).0, 0);
}
