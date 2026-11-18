
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn audit(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_miaudit")).current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn repo() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    assert_eq!(audit(d.path(), &["init"]).0, 0);
    d
}

fn audit_commit(d: &Path, files: &[(&str, &str)]) -> String {
    for (n, c) in files {
        fs::write(d.join(n), c).unwrap();
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
    let mut args = vec!["add"];
    args.extend(&names);
    assert_eq!(audit(d, &args).0, 0);
    let (code, v) = audit(d, &["commit", "-m", "change"]);
    assert_eq!(code, 0, "{v}");
    v["result"]["audit_id"].as_str().unwrap().to_string()
}

#[test]
fn bind_full_correspondence_with_github_link() {
    let d = repo();
    git(d.path(), &["remote", "add", "origin", "https://user:s3cret@github.com/acme/repo.git"]);
    let id = audit_commit(d.path(), &[("a.txt", "one"), ("b.txt", "two")]);
    git(d.path(), &["add", "a.txt", "b.txt"]);
    git(d.path(), &["commit", "-q", "-m", "c1"]);
    let sha = git(d.path(), &["rev-parse", "HEAD"]);

    let (code, v) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!(code, 0, "{v}");
    let r = &v["result"];
    assert_eq!(r["correspondence"], "full");
    assert_eq!((r["paths_matched"].as_u64(), r["paths_total"].as_u64()), (Some(2), Some(2)));
    assert_eq!(r["git_commit"], sha.as_str());
    assert_eq!(r["commit_web_url"], format!("https://github.com/acme/repo/commit/{sha}").as_str());
    let stored = fs::read_dir(d.path().join(".audit/bindings")).unwrap().map(|e| fs::read_to_string(e.unwrap().path()).unwrap()).collect::<String>();
    assert!(!stored.contains("s3cret") && !stored.contains("user:"), "{stored}");
}

#[test]
fn bind_is_idempotent_and_queryable_both_ways() {
    let d = repo();
    let id = audit_commit(d.path(), &[("a.txt", "one")]);
    git(d.path(), &["add", "a.txt"]);
    git(d.path(), &["commit", "-q", "-m", "c1"]);
    let (_, first) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    let (_, again) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!(first["result"]["binding_id"], again["result"]["binding_id"]);
    assert_eq!((first["result"]["duplicate"].as_bool(), again["result"]["duplicate"].as_bool()), (Some(false), Some(true)));
    assert_eq!(audit(d.path(), &["bindings", &id]).1["result"]["bindings"].as_array().unwrap().len(), 1);

    let short = git(d.path(), &["rev-parse", "--short", "HEAD"]);
    let rel = audit(d.path(), &["related", &short]).1;
    assert_eq!(rel["result"]["audits"][0]["audit_id"], id.as_str());
    assert_eq!(rel["result"]["audits"][0]["description"], "change");
    assert_eq!(audit(d.path(), &["show", &id]).1["result"]["git_bindings"][0]["body"]["correspondence"], "full");
    assert_eq!(audit(d.path(), &["log"]).1["result"]["commits"][0]["git"][0]["correspondence"], "full");
}

#[test]
fn partial_when_git_commit_has_only_some_changes_or_they_changed_later() {
    let d = repo();
    let id = audit_commit(d.path(), &[("a.txt", "one"), ("b.txt", "two")]);
    git(d.path(), &["add", "a.txt"]);
    git(d.path(), &["commit", "-q", "-m", "only a"]);
    let (code, v) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["correspondence"], "partial");
    let states: Vec<String> = v["result"]["paths"].as_array().unwrap().iter().map(|p| format!("{}={}", p["path"], p["state"])).collect();
    assert_eq!(states, vec!["\"a.txt\"=\"match\"", "\"b.txt\"=\"absent_in_git\""]);

    fs::write(d.path().join("a.txt"), "edited later").unwrap();
    git(d.path(), &["add", "a.txt", "b.txt"]);
    git(d.path(), &["commit", "-q", "-m", "edit a"]);
    let (code, v) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["result"]["correspondence"].as_str(), v["result"]["paths"][0]["state"].as_str()), (Some("partial"), Some("content_differs")));
}

#[test]
fn mismatch_is_rejected_unless_explicitly_asserted() {
    let d = repo();
    let id = audit_commit(d.path(), &[("a.txt", "one")]);
    fs::write(d.path().join("other.txt"), "x").unwrap();
    git(d.path(), &["add", "other.txt"]);
    git(d.path(), &["commit", "-q", "-m", "unrelated"]);
    let (code, v) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!((code, v["error"]["code"].as_str()), (3, Some("BINDING_MISMATCH")));
    assert_eq!(audit(d.path(), &["bindings"]).1["result"]["bindings"].as_array().unwrap().len(), 0);

    let (code, v) = audit(d.path(), &["bind", &id, "--git-commit", "HEAD", "--assert"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["correspondence"], "asserted");
    let b = &audit(d.path(), &["bindings", &id]).1["result"]["bindings"][0]["body"];
    assert_eq!(b["provenance"], "self_reported");
}

#[test]
fn deletions_are_verified_as_absent_in_git() {
    let d = repo();
    fs::write(d.path().join("gone.txt"), "x").unwrap();
    git(d.path(), &["add", "gone.txt"]);
    git(d.path(), &["commit", "-q", "-m", "add"]);
    let first = audit_commit(d.path(), &[("gone.txt", "x")]);
    fs::remove_file(d.path().join("gone.txt")).unwrap();
    assert_eq!(audit(d.path(), &["add", "gone.txt"]).0, 0);
    let (_, v) = audit(d.path(), &["commit", "-m", "delete"]);
    let del = v["result"]["audit_id"].as_str().unwrap().to_string();
    let (code, v) = audit(d.path(), &["bind", &del, "--git-commit", "HEAD"]);
    assert_eq!((code, v["error"]["code"].as_str()), (3, Some("BINDING_MISMATCH")));
    git(d.path(), &["rm", "-q", "gone.txt"]);
    git(d.path(), &["commit", "-q", "-m", "remove"]);
    let (code, v) = audit(d.path(), &["bind", &del, "--git-commit", "HEAD"]);
    assert_eq!((code, v["result"]["correspondence"].as_str()), (0, Some("full")), "{v}");
    let _ = first;
}

#[test]
fn rejects_unknown_commits_option_injection_and_unverifiable_audits() {
    let d = repo();
    let id = audit_commit(d.path(), &[("a.txt", "one")]);
    git(d.path(), &["add", "a.txt"]);
    git(d.path(), &["commit", "-q", "-m", "c1"]);
    for rev in ["deadbeefdeadbeefdeadbeefdeadbeefdeadbeef", "--upload-pack=touch /tmp/pwned", "-h", "no-such-branch", ""] {
        let arg = format!("--git-commit={rev}");
        let (code, v) = audit(d.path(), &["bind", &id, &arg]);
        assert_eq!((code, v["error"]["code"].as_str()), (2, Some("GIT_COMMIT_NOT_FOUND")), "{rev:?}");
    }
    assert_eq!(audit(d.path(), &["bind", "audit_nope", "--git-commit", "HEAD"]).1["error"]["code"], "OBJECT_NOT_FOUND");

    let f = d.path().join("c.json");
    fs::write(&f, r#"{"schema_version":"1","description":"only context"}"#).unwrap();
    audit(d.path(), &["context", "add", "--file", f.to_str().unwrap()]);
    let (_, v) = audit(d.path(), &["commit", "-m", "context only"]);
    let ctx_only = v["result"]["audit_id"].as_str().unwrap().to_string();
    let (code, v) = audit(d.path(), &["bind", &ctx_only, "--git-commit", "HEAD"]);
    assert_eq!((code, v["error"]["code"].as_str()), (4, Some("BINDING_UNVERIFIABLE")));
}

#[test]
fn verify_detects_a_tampered_binding_and_the_manifest_is_not_touched() {
    let d = repo();
    let id = audit_commit(d.path(), &[("a.txt", "one")]);
    git(d.path(), &["add", "a.txt"]);
    git(d.path(), &["commit", "-q", "-m", "c1"]);
    let before = fs::read(d.path().join(format!(".audit/commits/{id}.json"))).unwrap();
    audit(d.path(), &["bind", &id, "--git-commit", "HEAD"]);
    assert_eq!(fs::read(d.path().join(format!(".audit/commits/{id}.json"))).unwrap(), before, "binding must not alter the manifest");
    assert_eq!(audit(d.path(), &["verify", &id]).0, 0);

    let f = fs::read_dir(d.path().join(".audit/bindings")).unwrap().next().unwrap().unwrap().path();
    let text = fs::read_to_string(&f).unwrap().replace("\"full\"", "\"asserted\"");
    fs::write(&f, text).unwrap();
    let (code, v) = audit(d.path(), &["verify", &id]);
    assert_eq!((code, v["result"]["verified"]["integrity"].as_str()), (2, Some("fail")));
}
