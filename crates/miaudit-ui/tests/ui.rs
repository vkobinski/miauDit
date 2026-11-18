use miaudit_ui::{AppState, router};
use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use std::{fs, path::{Path, PathBuf}, process::Command, sync::Once};
use tempfile::TempDir;
use tower::ServiceExt;

const PORT: u16 = 7070;
const XSS: &str = "<script>alert(1)</script> & \"quotes\"";

fn audit_bin() -> PathBuf {
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo).args(["build", "-q", "-p", "miaudit-cli"]).current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    });
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/miaudit")
}

fn run(dir: &Path, prog: &str, args: &[&str]) -> String {
    let out = Command::new(prog).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(dir).args(args).output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}
fn git(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
    full.extend(args);
    run(dir, "git", &full).trim().to_string()
}

fn fixture() -> (TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let a = audit_bin();
    let a = a.to_str().unwrap();
    git(p, &["init", "-q"]);
    git(p, &["remote", "add", "origin", "git@github.com:acme/repo.git"]);
    run(p, a, &["init"]);
    fs::write(p.join("f.txt"), "hello").unwrap();
    run(p, a, &["add", "f.txt"]);
    fs::write(p.join("c.json"), format!(r#"{{"schema_version":"1","description":{},"source_refs":["policy://x"]}}"#, serde_json::to_string(XSS).unwrap())).unwrap();
    run(p, a, &["context", "add", "--file", "c.json"]);
    let out = run(p, a, &["--json", "commit", "-m", XSS]);
    let id = serde_json::from_str::<serde_json::Value>(&out).unwrap()["result"]["audit_id"].as_str().unwrap().to_string();
    git(p, &["add", "f.txt"]);
    git(p, &["commit", "-q", "-m", "c1"]);
    run(p, a, &["bind", &id, "--git-commit", "HEAD"]);
    (d, id)
}

fn app(d: &TempDir) -> axum::Router {
    router(AppState { repo: d.path().canonicalize().unwrap(), port: PORT, audit_bin: audit_bin() })
}

async fn get(app: &axum::Router, method: &str, uri: &str, host: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let req = Request::builder().method(method).uri(uri).header("host", host).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).to_string())
}

const HOST: &str = "127.0.0.1:7070";

#[tokio::test]
async fn pages_render_and_escape_untrusted_content() {
    let (d, id) = fixture();
    let app = app(&d);
    for uri in ["/".to_string(), format!("/audit/{id}")] {
        let (s, _, body) = get(&app, "GET", &uri, HOST).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(!body.contains("<script>alert(1)"), "unescaped script on {uri}");
        assert!(body.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quotes&quot;"), "{uri}");
    }
    let (_, _, detail) = get(&app, "GET", &format!("/audit/{id}"), HOST).await;
    for want in ["self-reported", "unverified", "integrity: pass", "none recorded", "not evaluated", "full match", "https://github.com/acme/repo/commit/", "match", "CAPTURE_UNAVAILABLE:agent_topology", "@test-user", "GitHub user"] {
        assert!(detail.contains(want), "missing {want:?}");
    }
    let (_, _, index) = get(&app, "GET", "/", HOST).await;
    assert!(index.contains("https://github.com/acme/repo/commit/") && index.contains("full match"));
}

#[tokio::test]
async fn rejects_foreign_hosts_and_non_get_methods() {
    let (d, id) = fixture();
    let app = app(&d);
    for host in ["evil.com", "evil.com:7070", "127.0.0.1:9999", "127.0.0.1", ""] {
        assert_eq!(get(&app, "GET", "/", host).await.0, StatusCode::FORBIDDEN, "{host:?}");
    }
    assert_eq!(get(&app, "GET", "/", "localhost:7070").await.0, StatusCode::OK);
    for m in ["POST", "PUT", "DELETE", "PATCH"] {
        assert_eq!(get(&app, m, "/", HOST).await.0, StatusCode::METHOD_NOT_ALLOWED, "{m}");
        assert_eq!(get(&app, m, &format!("/audit/{id}"), HOST).await.0, StatusCode::METHOD_NOT_ALLOWED, "{m}");
    }
}

#[tokio::test]
async fn sets_security_headers_and_rejects_bad_ids() {
    let (d, _) = fixture();
    let app = app(&d);
    let (_, h, _) = get(&app, "GET", "/", HOST).await;
    assert!(h["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["cache-control"], "no-store");
    for bad in ["/audit/nope", "/audit/audit_..%2F..%2Fconfig", "/audit/audit_AAAA", "/api/audits/../../etc/passwd", "/nothing"] {
        assert_eq!(get(&app, "GET", bad, HOST).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
    let unknown = format!("/audit/audit_{}", "0".repeat(64));
    assert_eq!(get(&app, "GET", &unknown, HOST).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn json_api_and_tampering_is_visible() {
    let (d, id) = fixture();
    let app = app(&d);
    let (s, h, body) = get(&app, "GET", "/api/audits", HOST).await;
    assert_eq!((s, h["content-type"].to_str().unwrap()), (StatusCode::OK, "application/json"));
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["audits"][0]["audit_id"], id.as_str());
    assert_eq!(v["audits"][0]["git"][0]["correspondence"], "full");
    let (s, _, raw) = get(&app, "GET", &format!("/api/audits/{id}"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    assert!(raw.contains("\"snapshot\""));

    let path = d.path().join(format!(".miaudit/commits/{id}.json"));
    let text = fs::read_to_string(&path).unwrap().replace("staged_paths_only", "staged_paths_onlx");
    fs::write(&path, text).unwrap();
    let (_, _, detail) = get(&app, "GET", &format!("/audit/{id}"), HOST).await;
    assert!(detail.contains("integrity: fail"), "tampering not shown");
}

#[tokio::test]
async fn uninitialized_repo_reports_a_clear_error() {
    let d = tempfile::tempdir().unwrap();
    let (s, _, body) = get(&app(&d), "GET", "/", HOST).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("not an initialized audit repository"));
}

#[tokio::test]
async fn screenshot_evidence_is_rendered_and_only_real_images_are_served() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let a = audit_bin();
    let a = a.to_str().unwrap();
    run(p, a, &["init"]);
    fs::write(p.join("f.txt"), "x").unwrap();
    run(p, a, &["add", "f.txt"]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend_from_slice(b"pixels");
    fs::write(p.join("shot.png"), &png).unwrap();
    fs::write(p.join("svg.png"), "<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>").unwrap();
    fs::write(p.join("e1.json"), r#"{"schema_version":"1","classification":"internal","description":"screenshot","file":"shot.png","phase":"after","environment":"staging"}"#).unwrap();
    fs::write(p.join("e2.json"), r#"{"schema_version":"1","classification":"internal","description":"fake","file":"svg.png"}"#).unwrap();
    let e1 = serde_json::from_str::<serde_json::Value>(&run(p, a, &["--json", "evidence", "add", "--file", "e1.json"])).unwrap()["result"]["id"].as_str().unwrap().to_string();
    let e2 = serde_json::from_str::<serde_json::Value>(&run(p, a, &["--json", "evidence", "add", "--file", "e2.json"])).unwrap()["result"]["id"].as_str().unwrap().to_string();
    let id = serde_json::from_str::<serde_json::Value>(&run(p, a, &["--json", "commit", "-m", "shots"])).unwrap()["result"]["audit_id"].as_str().unwrap().to_string();
    let app = app(&d);

    let (s, _, page) = get(&app, "GET", &format!("/audit/{id}"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains(&format!("src=\"/audit/{id}/evidence/{e1}/image\"")));
    assert!(!page.contains(&format!("evidence/{e2}/image")));
    assert!(page.contains("Staging") && page.contains("After"));

    let req = Request::builder().uri(format!("/audit/{id}/evidence/{e1}/image")).header("host", HOST).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert!(res.headers()["content-security-policy"].to_str().unwrap().contains("img-src 'self'"));
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().to_vec(), png);

    let (s, _, _) = get(&app, "GET", &format!("/audit/{id}/evidence/{e2}/image"), HOST).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "content that is not an image must never be served");
    let (s, _, _) = get(&app, "GET", &format!("/audit/{id}/evidence/evd_{}/image", "0".repeat(64)), HOST).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, "GET", &format!("/audit/{id}/evidence/{e1}/image"), "evil.example").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
