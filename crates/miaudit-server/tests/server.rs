use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use miaudit_server::{AppState, db::Db, router};
use serde_json::Value;
use std::{fs, path::{Path, PathBuf}, process::Command, sync::Once};
use tempfile::TempDir;
use tower::ServiceExt;

const XSS: &str = "<script>alert(1)</script> & \"quotes\"";
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4, 5, 6, 7, 8];

fn miaudit_bin() -> PathBuf {
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo).args(["build", "-q", "-p", "miaudit-cli"]).current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    });
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/miaudit")
}

fn run(dir: &Path, prog: &str, args: &[&str]) -> (String, i32) {
    let out = Command::new(prog).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).args(args).output().unwrap();
    (String::from_utf8(out.stdout).unwrap(), out.status.code().unwrap())
}

fn git(dir: &Path, args: &[&str]) {
    let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
    full.extend(args);
    run(dir, "git", &full);
}

struct Fixture {
    dir: TempDir,
    audit_id: String,
}

fn fixture(remote: &str, description: &str) -> Fixture {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let bin = miaudit_bin();
    let a = bin.to_str().unwrap();
    git(p, &["init", "-q"]);
    git(p, &["remote", "add", "origin", remote]);
    run(p, a, &["init"]);
    fs::write(p.join("f.txt"), description).unwrap();
    fs::write(p.join("shot.png"), PNG).unwrap();
    run(p, a, &["add", "f.txt", "shot.png"]);
    fs::write(p.join("c.json"), format!(r#"{{"schema_version":"1","description":{},"change_kind":"frontend"}}"#, serde_json::to_string(description).unwrap())).unwrap();
    run(p, a, &["context", "add", "--file", "c.json"]);
    let status: Value = serde_json::from_str(&run(p, a, &["--json", "status"]).0).unwrap();
    let snap = status["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    fs::write(p.join("d.json"), format!(r#"{{"schema_version":"1","question":"q","choice":"c","alternatives":["x"],"rationale":"r","risks":[],"snapshot_id":"{snap}"}}"#)).unwrap();
    run(p, a, &["decision", "add", "--file", "d.json"]);
    fs::write(p.join("e.json"), r#"{"schema_version":"1","classification":"internal","description":"after screenshot","file":"shot.png","phase":"after","environment":"local"}"#).unwrap();
    run(p, a, &["evidence", "add", "--file", "e.json"]);
    let out: Value = serde_json::from_str(&run(p, a, &["--json", "commit", "-m", description]).0).unwrap();
    let audit_id = out["result"]["audit_id"].as_str().unwrap().to_string();
    git(p, &["add", "f.txt", "shot.png"]);
    git(p, &["commit", "-q", "-m", "c1"]);
    run(p, a, &["bind", &audit_id, "--git-commit", "HEAD"]);
    Fixture { dir: d, audit_id }
}

async fn db_name_of(db: &Db) -> String {
    db.name().to_string()
}

async fn test_db() -> Option<Db> {
    let Ok(uri) = std::env::var("MIAUDIT_TEST_MONGO_URI") else {
        eprintln!("SKIPPED: MIAUDIT_TEST_MONGO_URI is not set, no MongoDB to test against");
        return None;
    };
    let name = format!("miaudit_test_{}", uuid_like());
    Some(Db::connect(&uri, &name).await.expect("connect to test MongoDB"))
}

fn uuid_like() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

struct Live {
    addr: String,
    token: String,
}

async fn serve(db: &Db, password: Option<&str>) -> Live {
    let token = format!("mdt_{}", uuid_like());
    db.create_token(&format!("t{}", uuid_like()), &token).await.unwrap();
    let app = router(AppState { db: db.clone(), read_password: password.map(String::from) });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Live { addr, token }
}

fn push(f: &Fixture, live: &Live, token: &str, name: Option<&str>) -> (Value, i32) {
    let bin = miaudit_bin();
    let mut args = vec!["--json", "push"];
    if let Some(n) = name {
        args.extend(["--name", n]);
    }
    let out = Command::new(bin).env("MIAUDIT_SERVER", &live.addr).env("MIAUDIT_TOKEN", token).current_dir(f.dir.path()).args(args).output().unwrap();
    (serde_json::from_slice(&out.stdout).unwrap(), out.status.code().unwrap())
}

async fn get(app: &axum::Router, uri: &str, auth: Option<&str>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(a) = auth {
        req = req.header("authorization", a);
    }
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let (s, h) = (res.status(), res.headers().clone());
    (s, h, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn text(b: Vec<u8>) -> String {
    String::from_utf8_lossy(&b).to_string()
}

fn app(db: &Db) -> axum::Router {
    router(AppState { db: db.clone(), read_password: None })
}

async fn raw_put(live: &Live, token: &str, path: &str, body: Vec<u8>) -> (u16, Value) {
    let url = format!("{}{path}", live.addr);
    let token = token.to_string();
    tokio::task::spawn_blocking(move || {
        let r = ureq::put(&url).set("Authorization", &format!("Bearer {token}")).set("Content-Type", "application/json").send_bytes(&body);
        match r {
            Ok(r) => (r.status(), r.into_json().unwrap_or(Value::Null)),
            Err(ureq::Error::Status(c, r)) => (c, r.into_json().unwrap_or(Value::Null)),
            Err(e) => panic!("{e}"),
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn push_then_browse_like_github() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let f = fixture("git@github.com:acme/web.git", XSS);

    let (out, code) = push(&f, &live, &live.token, None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["result"]["repo"], "acme/web");
    assert_eq!(out["result"]["sent"]["commits"], 1);
    assert_eq!(out["result"]["sent"]["blobs"], 1);
    assert_eq!(out["result"]["sent"]["bindings"], 1);

    let (again, code) = push(&f, &live, &live.token, None);
    assert_eq!(code, 0);
    assert_eq!(again["result"]["sent"], serde_json::json!({"blobs": 0, "objects": 0, "commits": 0, "bindings": 0}), "second push must send nothing");
    assert_eq!(again["result"]["already_on_server"]["commits"], 1);

    let app = app(&db);
    let (s, h, body) = get(&app, "/api/v1/config", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["mode"], "server");
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'self'") && !csp.contains("unsafe-inline"), "{csp}");

    let repos = |q: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/repos{q}");
        async move { serde_json::from_slice::<Value>(&get(&app, &uri, None).await.2).unwrap()["repos"].as_array().unwrap().clone() }
    };
    let all = repos("").await;
    assert_eq!((all[0]["slug"].as_str(), all[0]["audits"].as_i64()), (Some("acme/web"), Some(1)));
    assert_eq!(repos("?q=WEB").await.len(), 1, "search is case-insensitive");
    assert_eq!(repos("?q=nomatch").await.len(), 0);
    assert_eq!(repos("?q=.*").await.len(), 0, "search input must not be treated as a regex");

    let (s, _, body) = get(&app, "/acme/web/api/repo", None).await;
    assert_eq!(s, StatusCode::OK);
    let info: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!((info["repo"]["slug"].as_str(), info["authors"][0]["login"].as_str()), (Some("acme/web"), Some("test-user")));

    let (s, _, body) = get(&app, "/acme/web/api/audits", None).await;
    assert_eq!(s, StatusCode::OK);
    let list: Value = serde_json::from_slice(&body).unwrap();
    let row = &list["audits"][0];
    assert_eq!((list["total"].as_i64(), row["description"].as_str(), row["author"].as_str()), (Some(1), Some(XSS), Some("test-user")), "the API returns untrusted text untouched; the UI escapes it");
    assert_eq!(row["git"][0]["correspondence"], "full");
    assert!(row["gap_codes"].as_array().unwrap().iter().any(|c| c == "NO_HUMAN_APPROVAL"));

    let (s, h, body) = get(&app, &format!("/acme/web/api/audits/{}/detail", f.audit_id), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "application/json");
    let detail: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(detail["verification"]["integrity"], "pass");
    assert_eq!(detail["bindings"][0]["body"]["correspondence"], "full");
    assert_eq!(detail["decisions"][0]["object"]["provenance"], "self_reported");
    assert_eq!(detail["evidence"][0]["object"]["body"]["description"], "after screenshot");
    assert_eq!(detail["evidence"][0]["image"], true);
    assert!(detail["manifest"]["snapshot"].get("entries").is_none());

    let eid = detail["evidence"][0]["id"].as_str().unwrap().to_string();
    let (s, h, body) = get(&app, &format!("/acme/web/api/audits/{}/evidence/{eid}/image", f.audit_id), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "image/png");
    assert_eq!(body, PNG);

    let local = fs::read(f.dir.path().join(".miaudit/commits").join(format!("{}.json", f.audit_id))).unwrap();
    let (_, _, raw) = get(&app, &format!("/acme/web/api/audits/{}", f.audit_id), None).await;
    assert_eq!(raw, local, "the server must return the exact manifest bytes it was given");

    let total = |q: &str| {
        let app = app.clone();
        let uri = format!("/acme/web/api/audits{q}");
        async move { serde_json::from_slice::<Value>(&get(&app, &uri, None).await.2).unwrap()["total"].as_i64().unwrap() }
    };
    assert_eq!(total("?author=test-user").await, 1);
    assert_eq!(total("?author=nobody").await, 0);

    for uri in ["/", "/acme/web", &format!("/acme/web/audit/{}", f.audit_id)] {
        let (s, h, body) = get(&app, uri, None).await;
        let shell = text(body);
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(h["content-type"].to_str().unwrap().starts_with("text/html"), "{uri}");
        assert!(shell.contains(r#"id="root""#) && !shell.contains("<script>"), "{uri}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn repos_are_isolated_and_slugs_cannot_be_taken() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let a = fixture("git@github.com:acme/one.git", "audit of one");
    let b = fixture("git@github.com:acme/two.git", "audit of two");
    assert_eq!(push(&a, &live, &live.token, None).1, 0);
    assert_eq!(push(&b, &live, &live.token, None).1, 0);

    let app = app(&db);
    let (_, _, body) = get(&app, "/acme/one/api/audits", None).await;
    let page = text(body);
    assert!(page.contains("audit of one") && !page.contains("audit of two"));
    let (s, _, _) = get(&app, &format!("/acme/one/api/audits/{}/detail", b.audit_id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an audit of repo two must not open under repo one");
    let (s, _, _) = get(&app, &format!("/acme/one/api/audits/{}", b.audit_id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, _, body) = get(&app, "/api/v1/repos", None).await;
    let slugs: Vec<String> = serde_json::from_slice::<Value>(&body).unwrap()["repos"].as_array().unwrap().iter().map(|r| r["slug"].as_str().unwrap().to_string()).collect();
    assert!(slugs.contains(&"acme/one".to_string()) && slugs.contains(&"acme/two".to_string()) && slugs.len() == 2, "{slugs:?}");

    let c = fixture("git@github.com:someone/else.git", "third");
    let (out, code) = push(&c, &live, &live.token, Some("acme/one"));
    assert_eq!(code, 3, "{out}");
    assert_eq!(out["error"]["code"], "PUSH_CONFLICT");
}

#[tokio::test(flavor = "multi_thread")]
async fn server_rejects_bad_tokens_and_content_that_does_not_hash() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let f = fixture("git@github.com:acme/web.git", "plain");

    let (out, code) = push(&f, &live, "mdt_wrong", None);
    assert_eq!(code, 2, "{out}");
    assert_eq!(out["error"]["code"], "PUSH_UNAUTHORIZED");
    let (out, code) = push(&f, &live, "", None);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("NO_TOKEN")));

    assert_eq!(push(&f, &live, &live.token, None).1, 0);
    let repo_id = fs::read_to_string(f.dir.path().join(".miaudit/config.json")).unwrap();
    let repo_id = serde_json::from_str::<Value>(&repo_id).unwrap()["repo_id"].as_str().unwrap().to_string();

    let good = fs::read_to_string(f.dir.path().join(".miaudit/commits").join(format!("{}.json", f.audit_id))).unwrap();
    let tampered = good.replacen("plain", "tampered", 1);
    assert_ne!(good, tampered);
    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{repo_id}/commits/audit_{}", "1".repeat(64)), tampered.clone().into_bytes()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("ID_MISMATCH")));
    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{repo_id}/commits/{}", f.audit_id), tampered.into_bytes()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("ID_MISMATCH")));

    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{repo_id}/blobs/sha256:{}", "0".repeat(64)), b"not that".to_vec()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("DIGEST_MISMATCH")));

    let other_repo = format!("repo_{}", "e".repeat(32));
    let (code, _) = raw_put(&live, &live.token, &format!("/api/v1/repos/{other_repo}"), br#"{"slug":"x/y","remote_url":null}"#.to_vec()).await;
    assert_eq!(code, 200);
    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{other_repo}/commits/{}", f.audit_id), good.clone().into_bytes()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("REPO_MISMATCH")), "a manifest cannot be filed under another repo");

    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{repo_id}/commits/{}", f.audit_id), good.into_bytes()).await;
    assert_eq!((code, v["stored"].as_bool()), (200, Some(false)), "re-sending an identical commit is a no-op");

    let (code, v) = raw_put(&live, &live.token, "/api/v1/repos/repo_zzz", br#"{"slug":"a/b"}"#.to_vec()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("BAD_ID")));
    let (code, v) = raw_put(&live, &live.token, &format!("/api/v1/repos/{repo_id}"), br#"{"slug":"../../etc"}"#.to_vec()).await;
    assert_eq!((code, v["error"]["code"].as_str()), (422, Some("BAD_SLUG")));
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_token_stops_working_and_read_password_gates_the_ui() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, Some("hunter2")).await;
    let f = fixture("git@github.com:acme/web.git", "gated");
    assert_eq!(push(&f, &live, &live.token, None).1, 0);

    let app = router(AppState { db: db.clone(), read_password: Some("hunter2".into()) });
    let (s, h, _) = get(&app, "/", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h["www-authenticate"].to_str().unwrap().starts_with("Basic"));
    let b64 = |s: &str| {
        use base64::Engine;
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(s))
    };
    assert_eq!(get(&app, "/", Some(&b64("me:wrong"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(get(&app, "/", Some(&b64("me:hunter2"))).await.0, StatusCode::OK);
    assert_eq!(get(&app, "/api/v1/repos", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(get(&app, "/acme/web", None).await.0, StatusCode::UNAUTHORIZED, "the app shell is behind the password too");
    assert_eq!(get(&app, "/acme/web/api/audits", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(get(&app, "/acme/web", Some(&b64("me:hunter2"))).await.0, StatusCode::OK);
    assert_eq!(get(&app, "/healthz", None).await.0, StatusCode::OK);

    let tokens = db.list_tokens().await.unwrap();
    assert!(db.revoke_token(&tokens[0].name).await.unwrap());
    let (out, code) = push(&f, &live, &live.token, None);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("PUSH_UNAUTHORIZED")));
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_api_paths_and_bad_ids_are_404_but_app_routes_get_the_shell() {
    let Some(db) = test_db().await else { return };
    let app = app(&db);
    for bad in ["/acme/web/api/audits/nope", "/acme/web/api/audits/audit_..%2F..", "/a%20b/c/api/audits", "/api/x", "/acme/web/api/nothing", "/acme/web/api/audits", "/assets/missing.js"] {
        assert_eq!(get(&app, bad, None).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
    for spa in ["/nothing", "/no/such", "/acme/web/audit/nope"] {
        assert_eq!(get(&app, spa, None).await.0, StatusCode::OK, "{spa}: the React router renders its own not-found page");
    }
}

fn remote(live: &Live, args: &[&str]) -> (Value, i32) {
    let bin = miaudit_bin();
    let mut full = vec!["--json", "remote", "--server", live.addr.as_str()];
    full.extend(args);
    let out = Command::new(bin).current_dir(std::env::temp_dir()).args(full).output().unwrap();
    (serde_json::from_slice(&out.stdout).unwrap(), out.status.code().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_queries_repos_audits_and_filters_by_gap_and_author() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let a = fixture("git@github.com:acme/one.git", "audit of one");
    let b = fixture("git@github.com:acme/two.git", "audit of two");
    assert_eq!(push(&a, &live, &live.token, None).1, 0);
    assert_eq!(push(&b, &live, &live.token, None).1, 0);

    let (out, code) = remote(&live, &["repos"]);
    assert_eq!(code, 0, "{out}");
    let slugs: Vec<&str> = out["result"]["repos"].as_array().unwrap().iter().map(|r| r["slug"].as_str().unwrap()).collect();
    assert!(slugs.contains(&"acme/one") && slugs.contains(&"acme/two"), "{slugs:?}");

    let (out, _) = remote(&live, &["audits", "acme/one"]);
    assert_eq!(out["result"]["total"], 1);
    let row = &out["result"]["audits"][0];
    assert_eq!(row["audit_id"], a.audit_id.as_str());
    assert_eq!(row["author"], "test-user");
    assert!(row["gap_codes"].as_array().unwrap().iter().any(|c| c == "NO_HUMAN_APPROVAL"));
    assert_eq!(row["git"][0]["correspondence"], "full");

    let (out, _) = remote(&live, &["audits", "acme/one", "--gap", "CAPTURE_UNAVAILABLE"]);
    assert_eq!(out["result"]["total"], 1, "the fixture has no hook events, so every capture capability is unavailable");
    let (out, _) = remote(&live, &["audits", "acme/one", "--gap", "CAPTURE_PARTIAL"]);
    assert_eq!(out["result"]["total"], 0);
    let (out, _) = remote(&live, &["audits", "acme/one", "--gap", "NO_SUCH.*"]);
    assert_eq!(out["result"]["total"], 0, "the gap filter must not be treated as a regex");
    let (out, _) = remote(&live, &["audits", "acme/one", "--author", "test-user"]);
    assert_eq!(out["result"]["total"], 1);
    let (out, _) = remote(&live, &["audits", "acme/one", "--author", "nobody"]);
    assert_eq!(out["result"]["total"], 0);
    let (out, _) = remote(&live, &["audits", "acme/one", "--all"]);
    assert_eq!(out["result"]["audits"].as_array().unwrap().len(), 1);

    let (out, code) = remote(&live, &["show", "acme/one", &a.audit_id]);
    assert_eq!(code, 0);
    assert_eq!(out["result"]["manifest"]["description"], "audit of one");
    let lines = out["result"]["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l.as_str().unwrap().contains("CAPTURE_UNAVAILABLE:agent_topology")));

    let (out, code) = remote(&live, &["show", "acme/one", &b.audit_id]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("REMOTE_NOT_FOUND")), "an audit of repo two is not under repo one");
    let (out, code) = remote(&live, &["audits", "../etc"]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("BAD_REPO")));
    let (out, code) = remote(&live, &["audits", "acme/nothing"]);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("REMOTE_NOT_FOUND")));

    let bin = miaudit_bin();
    let text = Command::new(&bin).current_dir(std::env::temp_dir()).args(["remote", "--server", &live.addr, "audits", "acme/one"]).output().unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("1 audit commit(s) match in acme/one") && text.contains(&a.audit_id[..18]) && text.contains("@test-user"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_password_is_sent_by_the_remote_commands() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, Some("s3cret")).await;
    let bin = miaudit_bin();
    let run = |pw: Option<&str>| {
        let mut c = Command::new(&bin);
        c.current_dir(std::env::temp_dir()).args(["--json", "remote", "--server", &live.addr, "repos"]);
        if let Some(p) = pw {
            c.env("MIAUDIT_READ_PASSWORD", p);
        }
        let out = c.output().unwrap();
        (serde_json::from_slice::<Value>(&out.stdout).unwrap(), out.status.code().unwrap())
    };
    let (out, code) = run(None);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("REMOTE_UNAUTHORIZED")));
    let (_, code) = run(Some("wrong"));
    assert_eq!(code, 2);
    let (_, code) = run(Some("s3cret"));
    assert_eq!(code, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn audits_stored_before_gap_codes_existed_are_backfilled_on_open() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let f = fixture("git@github.com:acme/old.git", "old audit");
    assert_eq!(push(&f, &live, &live.token, None).1, 0);
    let uri = std::env::var("MIAUDIT_TEST_MONGO_URI").unwrap();
    let db_name = db_name_of(&db).await;
    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let audits = client.database(&db_name).collection::<mongodb::bson::Document>("audits");
    audits.update_many(mongodb::bson::doc! {}, mongodb::bson::doc! {"$unset": {"gap_codes": ""}}).await.unwrap();
    assert_eq!(audits.count_documents(mongodb::bson::doc! {"gap_codes": {"$exists": false}}).await.unwrap(), 1);
    Db::connect(&uri, &db_name).await.unwrap();
    assert_eq!(audits.count_documents(mongodb::bson::doc! {"gap_codes": {"$exists": false}}).await.unwrap(), 0);
    let (out, _) = remote(&live, &["audits", "acme/old", "--gap", "NO_HUMAN"]);
    assert_eq!(out["result"]["total"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_superseded_decision_is_reported_including_for_audits_stored_before_the_fields_existed() {
    let Some(db) = test_db().await else { return };
    let live = serve(&db, None).await;
    let f = fixture("git@github.com:acme/keys.git", "keys audit");
    let p = f.dir.path();
    let bin = miaudit_bin();
    let a = bin.to_str().unwrap();
    let shown: Value = serde_json::from_str(&run(p, a, &["--json", "show", &f.audit_id]).0).unwrap();
    let old = shown["result"]["objects"]["decisions"][0]["id"].as_str().unwrap().to_string();
    let snap = serde_json::from_str::<Value>(&run(p, a, &["--json", "status"]).0).unwrap()["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    fs::write(p.join("d2.json"), format!(r#"{{"schema_version":"1","question":"keys?","choice":"added later","alternatives":[],"rationale":"r","risks":[],"snapshot_id":"{snap}","supersedes":["{old}"]}}"#)).unwrap();
    let newer = serde_json::from_str::<Value>(&run(p, a, &["--json", "decision", "add", "--file", "d2.json"]).0).unwrap()["result"]["id"].as_str().unwrap().to_string();
    let second = serde_json::from_str::<Value>(&run(p, a, &["--json", "commit", "-m", "follow-up"]).0).unwrap()["result"]["audit_id"].as_str().unwrap().to_string();
    assert_eq!(push(&f, &live, &live.token, None).1, 0);

    let check = |app: axum::Router| {
        let (first, newer, second) = (f.audit_id.clone(), newer.clone(), second.clone());
        async move {
            let (s, _, body) = get(&app, &format!("/acme/keys/api/audits/{first}/detail"), None).await;
            assert_eq!(s, StatusCode::OK);
            let d: Value = serde_json::from_slice(&body).unwrap();
            let marker = &d["decisions"][0]["superseded_by"][0];
            assert_eq!((marker["decision_id"].as_str(), marker["audit_id"].as_str()), (Some(newer.as_str()), Some(second.as_str())));
            let (_, _, body) = get(&app, &format!("/acme/keys/api/audits/{second}/detail"), None).await;
            let d: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(d["decisions"][0]["superseded_by"].as_array().unwrap().len(), 0);
        }
    };
    check(app(&db)).await;

    let uri = std::env::var("MIAUDIT_TEST_MONGO_URI").unwrap();
    let name = db.name().to_string();
    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let database = client.database(&name);
    database.collection::<mongodb::bson::Document>("audits").update_many(mongodb::bson::doc! {}, mongodb::bson::doc! {"$unset": {"decision_ids": ""}}).await.unwrap();
    database.collection::<mongodb::bson::Document>("objects").update_many(mongodb::bson::doc! {}, mongodb::bson::doc! {"$unset": {"supersedes": ""}}).await.unwrap();
    let reopened = Db::connect(&uri, &name).await.unwrap();
    check(app(&reopened)).await;
}
