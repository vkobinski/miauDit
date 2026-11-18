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
    let (s, h, body) = get(&app, "/", None).await;
    let home = text(body);
    assert_eq!(s, StatusCode::OK);
    assert!(home.contains("/acme/web") && home.contains("1 audit commit"), "{home}");
    assert!(h["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));

    let (_, _, body) = get(&app, "/?q=WEB", None).await;
    assert!(text(body).contains("/acme/web"), "search is case-insensitive");
    let (_, _, body) = get(&app, "/?q=nomatch", None).await;
    assert!(text(body).contains("No repository matches"));
    let (_, _, body) = get(&app, "/?q=.*", None).await;
    assert!(text(body).contains("No repository matches"), "search input must not be treated as a regex");

    let (s, _, body) = get(&app, "/acme/web", None).await;
    let repo = text(body);
    assert_eq!(s, StatusCode::OK);
    assert!(!repo.contains("<script>alert(1)"), "unescaped script on repo page");
    assert!(repo.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quotes&quot;"));
    assert!(repo.contains("@test-user") && repo.contains("full match"));

    let (s, _, body) = get(&app, &format!("/acme/web/audit/{}", f.audit_id), None).await;
    let detail = text(body);
    assert_eq!(s, StatusCode::OK);
    for want in ["integrity: pass", "self-reported", "after screenshot", "Decisions", "full match", "/acme/web/api/audits/", "/acme/web/audit/"] {
        assert!(detail.contains(want), "missing {want:?}");
    }
    assert!(!detail.contains("<script>alert(1)"), "unescaped script on detail page");

    let eid = detail.split("/evidence/").nth(1).unwrap().split("/image").next().unwrap().to_string();
    let (s, h, body) = get(&app, &format!("/acme/web/audit/{}/evidence/{eid}/image", f.audit_id), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "image/png");
    assert_eq!(body, PNG);

    let local = fs::read(f.dir.path().join(".miaudit/commits").join(format!("{}.json", f.audit_id))).unwrap();
    let (_, _, raw) = get(&app, &format!("/acme/web/api/audits/{}", f.audit_id), None).await;
    assert_eq!(raw, local, "the server must return the exact manifest bytes it was given");

    let (_, _, body) = get(&app, "/api/v1/repos", None).await;
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["repos"][0]["slug"], "acme/web");
    assert_eq!(v["repos"][0]["audits"], 1);

    let (_, _, body) = get(&app, "/acme/web?author=test-user", None).await;
    assert!(text(body).contains("1 audit commit"));
    let (_, _, body) = get(&app, "/acme/web?author=nobody", None).await;
    assert!(text(body).contains("No audit commits match"));
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
    let (_, _, body) = get(&app, "/acme/one", None).await;
    let page = text(body);
    assert!(page.contains("audit of one") && !page.contains("audit of two"));
    let (s, _, _) = get(&app, &format!("/acme/one/audit/{}", b.audit_id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an audit of repo two must not open under repo one");
    let (_, _, body) = get(&app, "/", None).await;
    let home = text(body);
    assert!(home.contains("/acme/one") && home.contains("/acme/two") && home.contains("2 total"));

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
    assert_eq!(get(&app, "/healthz", None).await.0, StatusCode::OK);

    let tokens = db.list_tokens().await.unwrap();
    assert!(db.revoke_token(&tokens[0].name).await.unwrap());
    let (out, code) = push(&f, &live, &live.token, None);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("PUSH_UNAUTHORIZED")));
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_pages_and_bad_ids_are_404() {
    let Some(db) = test_db().await else { return };
    let app = app(&db);
    for bad in ["/nothing", "/no/such", "/acme/web/audit/nope", "/acme/web/audit/audit_..%2F..", "/a%20b/c", "/api/x"] {
        assert_eq!(get(&app, bad, None).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
}
