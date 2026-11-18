use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_miaudit");

fn run(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(BIN).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).arg("--json").args(args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), v)
}

fn repo_with_audit() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run(d.path(), &["init", "--no-hooks"]).0, 0);
    fs::write(d.path().join("a.txt"), "one").unwrap();
    assert_eq!(run(d.path(), &["add", "a.txt"]).0, 0);
    let (code, v) = run(d.path(), &["commit", "-m", "first audit"]);
    assert_eq!(code, 0, "{v}");
    let id = v["result"]["audit_id"].as_str().unwrap().to_string();
    (d, id)
}

struct Viewer {
    child: Child,
    url: String,
    slug: String,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(cwd: &Path, args: &[&str]) -> Viewer {
    let mut child = Command::new(BIN).current_dir(cwd).arg("--json").args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(Duration::from_secs(30)).expect("the viewer printed nothing");
    let v: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("first line is not json: {line:?}"));
    Viewer { child, url: v["result"]["url"].as_str().unwrap().to_string(), slug: v["result"]["slug"].as_str().unwrap().to_string() }
}

fn get(url: &str) -> (u16, String) {
    match ureq::get(url).call() {
        Ok(r) => (r.status(), r.into_string().unwrap()),
        Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
        Err(e) => panic!("{url}: {e}"),
    }
}

#[test]
fn ui_serves_the_repository_found_from_a_subdirectory() {
    let (d, id) = repo_with_audit();
    let sub = d.path().join("deep/er");
    fs::create_dir_all(&sub).unwrap();
    let v = start(&sub, &["ui", "--port", "0"]);
    assert!(v.url.starts_with("http://127.0.0.1:"), "{}", v.url);
    assert!(v.slug.starts_with("local/"));

    let (s, body) = get(&format!("{}/api/v1/config", v.url));
    assert_eq!(s, 200);
    let c: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((c["mode"].as_str(), c["repo"].as_str()), (Some("local"), Some(v.slug.as_str())));

    let (_, body) = get(&format!("{}/{}/api/audits", v.url, v.slug));
    let list: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((list["total"].as_u64(), list["audits"][0]["audit_id"].as_str()), (Some(1), Some(id.as_str())));

    let (s, body) = get(&format!("{}/{}/api/audits/{id}/detail", v.url, v.slug));
    assert_eq!(s, 200);
    let detail: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["verification"]["state"], "result", "the viewer verifies by running this same executable");
    assert_eq!(detail["verification"]["integrity"], "pass");

    let (s, shell) = get(&format!("{}/{}", v.url, v.slug));
    assert_eq!(s, 200);
    assert!(shell.contains(r#"id="root""#) && !shell.contains("<script>"));
}

#[test]
fn ui_is_read_only_and_checks_the_host_header() {
    let (d, _) = repo_with_audit();
    let v = start(d.path(), &["ui", "--port", "0"]);
    match ureq::get(&format!("{}/api/v1/config", v.url)).set("Host", "evil.example").call() {
        Err(ureq::Error::Status(403, _)) => {}
        other => panic!("a foreign Host header must be refused: {other:?}"),
    }
    match ureq::post(&format!("{}/api/v1/repos", v.url)).send_string("x") {
        Err(ureq::Error::Status(405, _)) => {}
        other => panic!("POST must be refused: {other:?}"),
    }
}

#[test]
fn ui_accepts_an_explicit_repo_and_rejects_a_missing_one() {
    let (d, _) = repo_with_audit();
    let elsewhere = tempfile::tempdir().unwrap();
    let v = start(elsewhere.path(), &["ui", "--port", "0", "--repo", d.path().to_str().unwrap()]);
    let (_, body) = get(&format!("{}/api/v1/repos", v.url));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["repos"][0]["audits"], 1);
    drop(v);
    let (code, out) = run(elsewhere.path(), &["ui", "--port", "0", "--repo", "/definitely/not/here"]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("BAD_REPO")));
}

#[test]
fn ui_outside_a_store_and_on_a_busy_port_fail_with_clear_errors() {
    let empty = tempfile::tempdir().unwrap();
    let (code, out) = run(empty.path(), &["ui", "--port", "0"]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("NOT_INITIALIZED")));

    let (d, _) = repo_with_audit();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port().to_string();
    let (code, out) = run(d.path(), &["ui", "--port", &port]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("UI_BIND_FAILED")), "{out}");
}

#[test]
fn ui_is_listed_in_help() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.lines().any(|l| l.trim_start().starts_with("ui ")), "{help}");
}
