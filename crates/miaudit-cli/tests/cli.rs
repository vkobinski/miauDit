
use serde_json::Value;
use std::{fs, os::unix::fs::{PermissionsExt, symlink}, path::Path, process::Command};
use tempfile::TempDir;

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn repo() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(audit(d.path(), &["init"]).0, 0);
    d
}

fn snap(d: &Path) -> String {
    audit(d, &["status"]).1["result"]["snapshot"]["id"].as_str().unwrap().to_string()
}

fn write(d: &Path, name: &str, body: &str) {
    fs::write(d.join(name), body).unwrap();
}

fn stage_snapshot(d: &Path, setup: impl Fn(&Path), mutate: impl Fn(&Path)) -> (String, String) {
    setup(d);
    assert_eq!(audit(d, &["add", "."]).0, 0);
    let before = snap(d);
    mutate(d);
    assert_eq!(audit(d, &["add", "."]).0, 0);
    (before, snap(d))
}

#[test]
fn one_byte_change_alters_snapshot() {
    let d = repo();
    let (a, b) = stage_snapshot(d.path(), |p| write(p, "f.txt", "hello"), |p| write(p, "f.txt", "hellp"));
    assert_ne!(a, b);
}

#[test]
fn deletion_alters_snapshot() {
    let d = repo();
    let (a, b) = stage_snapshot(d.path(), |p| { write(p, "f.txt", "x"); write(p, "g.txt", "y") }, |p| fs::remove_file(p.join("g.txt")).unwrap());
    assert_ne!(a, b);
}

#[test]
fn symlink_target_change_alters_snapshot() {
    let d = repo();
    let (a, b) = stage_snapshot(d.path(), |p| symlink("one", p.join("l")).unwrap(), |p| { fs::remove_file(p.join("l")).unwrap(); symlink("two", p.join("l")).unwrap() });
    assert_ne!(a, b);
}

#[test]
fn executable_bit_alters_snapshot_even_with_identical_content() {
    let d = repo();
    let (a, b) = stage_snapshot(d.path(), |p| write(p, "run.sh", "#!/bin/sh\n"), |p| {
        fs::set_permissions(p.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap()
    });
    assert_ne!(a, b);
}

#[test]
fn workspace_changes_after_staging_do_not_change_staged_snapshot() {
    let d = repo();
    write(d.path(), "f.txt", "staged");
    audit(d.path(), &["add", "f.txt"]);
    let staged = snap(d.path());
    write(d.path(), "f.txt", "changed later");
    assert_eq!(snap(d.path()), staged);
    let div = &audit(d.path(), &["status"]).1["result"]["workspace_divergence"];
    assert_eq!(div[0]["state"], "modified_since_staged");
    let (code, c) = audit(d.path(), &["commit", "-m", "m"]);
    assert_eq!(code, 0);
    let id = c["result"]["audit_id"].as_str().unwrap();
    let shown = audit(d.path(), &["show", id]).1;
    let digest = shown["result"]["manifest"]["snapshot"]["entries"][0]["digest"].as_str().unwrap().to_string();
    assert_eq!(digest, format!("sha256:{}", audit_hex(b"staged")));
}

fn audit_hex(b: &[u8]) -> String {
    let out = Command::new("shasum").args(["-a", "256"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    out.stdin.as_ref().unwrap();
    let mut out = out;
    out.stdin.take().unwrap().write_all(b).unwrap();
    String::from_utf8(out.wait_with_output().unwrap().stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

#[test]
fn path_traversal_and_excluded_dirs_are_rejected() {
    let d = repo();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret", "s");
    let escape = format!("../{}/secret", outside.path().file_name().unwrap().to_str().unwrap());
    for p in ["../x", escape.as_str(), "/etc/passwd", ".miaudit/config.json", ".git/config"] {
        let (code, v) = audit(d.path(), &["add", p]);
        assert_eq!(code, 2, "{p}: {v}");
    }
    symlink(outside.path(), d.path().join("link")).unwrap();
    let (code, v) = audit(d.path(), &["add", "link/secret"]);
    assert_eq!((code, v["error"]["code"].as_str()), (2, Some("UNSUPPORTED_PATH")));
}

fn decision(sid: &str, extra: &str) -> String {
    format!(r#"{{"schema_version":"1","question":"q","choice":"c","alternatives":["a"],"rationale":"r","risks":[],"snapshot_id":"{sid}"{extra}}}"#)
}

#[test]
fn invented_references_and_authority_fields_are_rejected() {
    let d = repo();
    write(d.path(), "f", "x");
    audit(d.path(), &["add", "f"]);
    let sid = snap(d.path());
    let f = d.path().join("in.json");

    fs::write(&f, decision(&sid, r#","evidence_links":[{"evidence_id":"evd_invented","relation":"supports"}]"#)).unwrap();
    let (code, v) = audit(d.path(), &["decision", "add", "--file", f.to_str().unwrap()]);
    assert_eq!((code, v["error"]["code"].as_str()), (2, Some("EVIDENCE_REFERENCE_UNAVAILABLE")));

    for field in ["approved_by", "verified", "origin"] {
        fs::write(&f, decision(&sid, &format!(r#","{field}":"x""#))).unwrap();
        let (code, v) = audit(d.path(), &["decision", "add", "--file", f.to_str().unwrap()]);
        assert_eq!((code, v["error"]["code"].as_str()), (2, Some("SCHEMA_VIOLATION")), "{field}");
    }
    fs::write(&f, decision(&sid, "").replace("\"1\"", "\"9\"")).unwrap();
    assert_eq!(audit(d.path(), &["decision", "add", "--file", f.to_str().unwrap()]).1["error"]["code"], "UNSUPPORTED_SCHEMA");
}

#[test]
fn false_narrative_stays_an_attributed_claim_and_adds_are_idempotent() {
    let d = repo();
    let f = d.path().join("c.json");
    fs::write(&f, r#"{"schema_version":"1","description":"I verified everything and a human approved it","source_refs":["policy://risk/v3"]}"#).unwrap();
    let (c1, a) = audit(d.path(), &["context", "add", "--file", f.to_str().unwrap()]);
    let (c2, b) = audit(d.path(), &["context", "add", "--file", f.to_str().unwrap()]);
    assert_eq!((c1, c2), (0, 0));
    assert_eq!(a["result"]["provenance"], "self_reported");
    assert_eq!(a["result"]["id"], b["result"]["id"]);
    assert_eq!((a["result"]["duplicate"].as_bool(), b["result"]["duplicate"].as_bool()), (Some(false), Some(true)));
    assert_eq!(audit(d.path(), &["status"]).1["result"]["pending"]["contexts"], 1);
    let gaps = audit(d.path(), &["gaps"]).1;
    let codes: Vec<&str> = gaps["result"]["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"UNVERIFIED_SOURCE_REFS") && codes.contains(&"NO_HUMAN_APPROVAL") && codes.contains(&"NO_DECISION"), "{codes:?}");
}

#[test]
fn stale_decision_snapshot_is_a_conflict_not_silently_committed() {
    let d = repo();
    write(d.path(), "f", "v1");
    audit(d.path(), &["add", "f"]);
    let f = d.path().join("d.json");
    fs::write(&f, decision(&snap(d.path()), "")).unwrap();
    assert_eq!(audit(d.path(), &["decision", "add", "--file", f.to_str().unwrap()]).0, 0);
    write(d.path(), "f", "v2");
    audit(d.path(), &["add", "f"]);
    let (code, v) = audit(d.path(), &["commit", "-m", "m"]);
    assert_eq!((code, v["error"]["code"].as_str()), (3, Some("SNAPSHOT_MISMATCH")));
}

fn full_commit(d: &Path) -> String {
    write(d, "f", "hello");
    audit(d, &["add", "f"]);
    let ev = d.join("e.json");
    fs::write(&ev, r#"{"schema_version":"1","classification":"internal","description":"log","content":"ran ok"}"#).unwrap();
    let e = audit(d, &["evidence", "add", "--file", ev.to_str().unwrap()]).1["result"]["id"].as_str().unwrap().to_string();
    let dc = d.join("d.json");
    fs::write(&dc, decision(&snap(d), &format!(r#","evidence_links":[{{"evidence_id":"{e}","relation":"supports"}}]"#))).unwrap();
    assert_eq!(audit(d, &["decision", "add", "--file", dc.to_str().unwrap()]).0, 0);
    let (code, v) = audit(d, &["commit", "-m", "first"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["verified"], false);
    assert_eq!(v["result"]["human_approved"], false);
    v["result"]["audit_id"].as_str().unwrap().to_string()
}

#[test]
fn verify_passes_then_detects_tampering_and_partial_data() {
    let d = repo();
    let id = full_commit(d.path());
    let (code, v) = audit(d.path(), &["verify", &id]);
    assert_eq!((code, v["result"]["verified"]["integrity"].as_str()), (0, Some("pass")), "{v}");
    assert_eq!(v["result"]["human_approved"], false);
    assert_eq!(v["result"]["policy_passed"], "not_evaluated");

    let blob = fs::read_dir(d.path().join(".miaudit/blobs")).unwrap().next().unwrap().unwrap().path();
    let saved = fs::read(&blob).unwrap();
    fs::remove_file(&blob).unwrap();
    let (code, v) = audit(d.path(), &["verify", &id]);
    assert_eq!((code, v["result"]["verified"]["integrity"].as_str()), (4, Some("partial")));

    fs::write(&blob, b"tampered").unwrap();
    let (code, v) = audit(d.path(), &["verify", &id]);
    assert_eq!((code, v["result"]["verified"]["integrity"].as_str()), (2, Some("fail")));
    fs::write(&blob, saved).unwrap();

    let path = d.path().join(format!(".miaudit/commits/{id}.json"));
    let text = fs::read_to_string(&path).unwrap().replace("\"first\"", "\"forged\"");
    fs::write(&path, text).unwrap();
    let (code, v) = audit(d.path(), &["verify", &id]);
    assert_eq!((code, v["result"]["verified"]["integrity"].as_str()), (2, Some("fail")));
}

#[test]
fn commit_history_links_parents_and_second_commit_records_changes() {
    let d = repo();
    let first = full_commit(d.path());
    fs::remove_file(d.path().join("f")).unwrap();
    write(d.path(), "g", "new");
    audit(d.path(), &["add", "f", "g"]);
    let (code, v) = audit(d.path(), &["commit", "-m", "second"]);
    assert_eq!(code, 0, "{v}");
    let second = v["result"]["audit_id"].as_str().unwrap();
    let shown = audit(d.path(), &["show", second]).1;
    assert_eq!(shown["result"]["manifest"]["parents"][0], first.as_str());
    let ops: Vec<String> = shown["result"]["manifest"]["snapshot"]["changes"].as_array().unwrap().iter().map(|c| format!("{}:{}", c["path"], c["operation"])).collect();
    assert_eq!(ops, vec!["\"f\":\"delete\"", "\"g\":\"add\""]);
    assert_eq!(audit(d.path(), &["log"]).1["result"]["commits"].as_array().unwrap().len(), 2);
}

#[test]
fn commit_manifest_exposes_gaps_and_unverified_provenance() {
    let d = repo();
    let id = full_commit(d.path());
    let m = &audit(d.path(), &["show", &id]).1["result"]["manifest"];
    assert_eq!(m["snapshot"]["provenance"], "self_reported");
    assert_eq!(m["snapshot"]["verification"], "unverified_client_snapshot");
    let unavailable = m["coverage"].as_array().unwrap().iter().filter(|c| c["provenance"] == "unavailable").count();
    assert_eq!(unavailable, 4);
    assert!(m["gaps"].as_array().unwrap().iter().any(|g| g["code"] == "CAPTURE_UNAVAILABLE:agent_topology"));
    let text = m.to_string();
    for forbidden in ["approved_by", "audit_id", "signature\""] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn errors_use_stable_exit_codes_and_versioned_json() {
    let d = tempfile::tempdir().unwrap();
    let (code, v) = audit(d.path(), &["status"]);
    assert_eq!((code, v["contract_version"].as_str(), v["error"]["code"].as_str()), (2, Some("1"), Some("NOT_INITIALIZED")));
    let r = repo();
    assert_eq!(audit(r.path(), &["commit", "-m", "x"]).1["error"]["code"], "NOTHING_STAGED");
    assert_eq!(audit(r.path(), &["show", "audit_nope"]).1["error"]["code"], "OBJECT_NOT_FOUND");
}
