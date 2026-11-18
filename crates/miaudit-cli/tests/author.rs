use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tempfile::TempDir;

fn run(dir: &Path, envs: &[(&str, &str)], clear: bool, args: &[&str]) -> (i32, Value) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_miaudit"));
    c.current_dir(dir).arg("--json").args(args);
    if clear {
        c.env_remove("MIAUDIT_GITHUB_USER");
    }
    for (k, v) in envs {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn repo() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run(d.path(), &[], true, &["init"]).0, 0);
    fs::write(d.path().join("f.txt"), "x").unwrap();
    assert_eq!(run(d.path(), &[], true, &["add", "f.txt"]).0, 0);
    d
}

fn fake_gh(body: &str) -> TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("gh");
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    d
}

fn author_of(d: &Path) -> Value {
    let id = run(d, &[], true, &["log"]).1["result"]["commits"][0]["audit_id"].as_str().unwrap().to_string();
    run(d, &[], true, &["show", &id]).1["result"]["manifest"]["author"].clone()
}

fn gap_codes(d: &Path) -> Vec<String> {
    let id = run(d, &[], true, &["log"]).1["result"]["commits"][0]["audit_id"].as_str().unwrap().to_string();
    run(d, &[], true, &["show", &id]).1["result"]["manifest"]["gaps"].as_array().unwrap().iter().map(|g| g["code"].as_str().unwrap().to_string()).collect()
}

#[test]
fn store_is_a_miaudit_directory_that_ignores_itself() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run(d.path(), &[], true, &["init"]).0, 0);
    assert!(d.path().join(".miaudit/config.json").is_file());
    assert!(!d.path().join(".audit").exists());
    assert_eq!(fs::read_to_string(d.path().join(".miaudit/.gitignore")).unwrap(), "*\n");
    fs::create_dir(d.path().join("sub")).unwrap();
    assert_eq!(run(&d.path().join("sub"), &[], true, &["status"]).0, 0);
}

#[test]
fn legacy_dot_audit_store_is_reported_not_silently_ignored() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".audit")).unwrap();
    fs::write(d.path().join(".audit/config.json"), "{}").unwrap();
    let (code, v) = run(d.path(), &[], true, &["status"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "LEGACY_STORE");
}

#[test]
fn author_from_gh_is_derived_and_recorded_with_numeric_id() {
    let d = repo();
    let gh = fake_gh(r#"echo '{"login":"octo-cat","id":4242,"email":"private@example.com"}'"#);
    let path = format!("{}:/usr/bin:/bin", gh.path().display());
    let (code, _) = run(d.path(), &[("PATH", &path)], true, &["commit", "-m", "m"]);
    assert_eq!(code, 0);
    let a = author_of(d.path());
    assert_eq!(a["github_login"], "octo-cat");
    assert_eq!(a["github_id"], 4242);
    assert_eq!(a["provenance"], "derived");
    assert_eq!(a["source"], "gh_api_user");
    assert!(!a.to_string().contains("private@example.com"));
    assert!(gap_codes(d.path()).contains(&"AUTHOR_UNSIGNED".to_string()));
}

#[test]
fn author_from_environment_is_self_reported() {
    let d = repo();
    assert_eq!(run(d.path(), &[("MIAUDIT_GITHUB_USER", "someone")], false, &["commit", "-m", "m"]).0, 0);
    let a = author_of(d.path());
    assert_eq!(a["github_login"], "someone");
    assert_eq!(a["provenance"], "self_reported");
    assert!(gap_codes(d.path()).contains(&"AUTHOR_ASSERTED".to_string()));
}

#[test]
fn missing_gh_and_failing_gh_record_an_unavailable_author_instead_of_guessing() {
    let empty = tempfile::tempdir().unwrap();
    let d = repo();
    let only = empty.path().display().to_string();
    assert_eq!(run(d.path(), &[("PATH", &only)], true, &["commit", "-m", "m"]).0, 0);
    let a = author_of(d.path());
    assert!(a["github_login"].is_null());
    assert_eq!(a["provenance"], "unavailable");
    assert!(gap_codes(d.path()).contains(&"AUTHOR_UNAVAILABLE".to_string()));

    let d2 = repo();
    let gh = fake_gh("exit 1");
    let path = format!("{}:/usr/bin:/bin", gh.path().display());
    assert_eq!(run(d2.path(), &[("PATH", &path)], true, &["commit", "-m", "m"]).0, 0);
    assert_eq!(author_of(d2.path())["provenance"], "unavailable");
}

#[test]
fn invalid_logins_are_never_recorded() {
    let d = repo();
    assert_eq!(run(d.path(), &[("MIAUDIT_GITHUB_USER", "bad user\"<x>")], false, &["commit", "-m", "m"]).0, 0);
    assert!(author_of(d.path())["github_login"].is_null());

    let d2 = repo();
    let gh = fake_gh(r#"echo '{"login":"<script>","id":1}'"#);
    let path = format!("{}:/usr/bin:/bin", gh.path().display());
    run(d2.path(), &[("PATH", &path)], true, &["commit", "-m", "m"]);
    assert!(author_of(d2.path())["github_login"].is_null());
}

#[test]
fn log_lists_the_author_and_two_users_give_two_ids() {
    let d = repo();
    run(d.path(), &[("MIAUDIT_GITHUB_USER", "alice")], false, &["commit", "-m", "a"]);
    let log = run(d.path(), &[], true, &["log"]).1;
    assert_eq!(log["result"]["commits"][0]["author"], "alice");
}
