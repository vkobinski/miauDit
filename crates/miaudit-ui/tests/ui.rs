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

fn slug(d: &TempDir) -> String {
    miaudit_ui::local_slug(&d.path().canonicalize().unwrap())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("not json ({e}): {body}"))
}

async fn get(app: &axum::Router, method: &str, uri: &str, host: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let req = Request::builder().method(method).uri(uri).header("host", host).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).to_string())
}

const HOST: &str = "127.0.0.1:7070";

#[tokio::test]
async fn api_serves_config_repo_audits_and_detail() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);

    let (s, h, body) = get(&app, "GET", "/api/v1/config", HOST).await;
    assert_eq!((s, h["content-type"].to_str().unwrap()), (StatusCode::OK, "application/json"));
    let c = json(&body);
    assert_eq!((c["mode"].as_str(), c["repo"].as_str()), (Some("local"), Some(slug.as_str())));

    let repos = json(&get(&app, "GET", "/api/v1/repos", HOST).await.2);
    assert_eq!(repos["repos"][0]["slug"], slug.as_str());
    assert_eq!(repos["repos"][0]["audits"], 1);
    assert!(repos["repos"][0]["repo_id"].as_str().unwrap().starts_with("repo_"));

    let info = json(&get(&app, "GET", &format!("/{slug}/api/repo"), HOST).await.2);
    assert_eq!(info["authors"][0]["login"], "test-user");

    let list = json(&get(&app, "GET", &format!("/{slug}/api/audits"), HOST).await.2);
    assert_eq!((list["total"].as_u64(), list["page"].as_u64(), list["per_page"].as_u64()), (Some(1), Some(1), Some(25)));
    let row = &list["audits"][0];
    assert_eq!(row["audit_id"], id.as_str());
    assert_eq!(row["author"], "test-user");
    assert_eq!(row["git"][0]["correspondence"], "full");
    assert!(row["git"][0]["url"].as_str().unwrap().starts_with("https://github.com/acme/repo/commit/"));
    assert!(row["gap_codes"].as_array().unwrap().iter().any(|c| c == "CAPTURE_UNAVAILABLE:agent_topology"));

    let (s, _, raw) = get(&app, "GET", &format!("/{slug}/api/audits/{id}"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    assert!(raw.contains("\"snapshot\""));

    let (s, _, body) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/detail"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    let d = json(&body);
    assert_eq!(d["verification"]["state"], "result");
    assert_eq!(d["verification"]["integrity"], "pass");
    assert_eq!(d["bindings"][0]["body"]["correspondence"], "full");
    assert_eq!(d["contexts"][0]["object"]["provenance"], "self_reported");
    assert_eq!(d["contexts"][0]["object"]["body"]["source_refs"][0], "policy://x");
    assert_eq!(d["manifest"]["snapshot"]["entries_count"], 1);
    assert!(d["manifest"]["snapshot"].get("entries").is_none(), "the full tree is not sent to the browser");
    assert_eq!(d["manifest"]["author"]["github_login"], "test-user");
}

#[tokio::test]
async fn untrusted_text_is_returned_as_data_with_safe_headers() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    let (s, h, body) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/detail"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "application/json");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let d = json(&body);
    assert_eq!(d["manifest"]["description"], XSS, "escaping is the renderer's job; the API must not alter the text");
    assert_eq!(d["contexts"][0]["object"]["body"]["description"], XSS);
}

#[tokio::test]
async fn app_routes_get_the_react_shell_with_no_inline_script() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    for uri in ["/".to_string(), format!("/{slug}"), format!("/{slug}/audit/{id}")] {
        let (s, h, body) = get(&app, "GET", &uri, HOST).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(h["content-type"].to_str().unwrap().starts_with("text/html"), "{uri}");
        assert!(body.contains(r#"id="root""#) && body.contains("/assets/"), "{uri}");
        assert!(!body.contains("<script>"), "an inline script would break the CSP: {uri}");
    }
    let (_, _, shell) = get(&app, "GET", "/", HOST).await;
    let js = shell.split("src=\"/assets/").nth(1).unwrap().split('"').next().unwrap();
    let (s, h, _) = get(&app, "GET", &format!("/assets/{js}"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h["content-type"].to_str().unwrap().contains("javascript"));
}

#[tokio::test]
async fn rejects_foreign_hosts_and_non_get_methods() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    for host in ["evil.com", "evil.com:7070", "127.0.0.1:9999", "127.0.0.1", ""] {
        for uri in ["/", "/api/v1/config"] {
            assert_eq!(get(&app, "GET", uri, host).await.0, StatusCode::FORBIDDEN, "{host:?} {uri}");
        }
    }
    assert_eq!(get(&app, "GET", "/", "localhost:7070").await.0, StatusCode::OK);
    for m in ["POST", "PUT", "DELETE", "PATCH"] {
        for uri in ["/".to_string(), "/api/v1/repos".to_string(), format!("/{slug}/api/audits/{id}/detail"), format!("/{slug}/audit/{id}")] {
            assert_eq!(get(&app, m, &uri, HOST).await.0, StatusCode::METHOD_NOT_ALLOWED, "{m} {uri}");
        }
    }
}

#[tokio::test]
async fn sets_a_strict_csp_and_rejects_bad_ids_and_other_repos() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    let (_, h, _) = get(&app, "GET", "/", HOST).await;
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'self'") && csp.contains("connect-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(!csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"), "{csp}");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["cache-control"], "no-store");
    for bad in [
        format!("/{slug}/api/audits/nope"),
        format!("/{slug}/api/audits/audit_..%2F..%2Fconfig"),
        format!("/{slug}/api/audits/audit_AAAA/detail"),
        format!("/{slug}/api/audits/../../etc/passwd"),
        format!("/other/repo/api/audits/{id}/detail"),
        "/other/repo/api/audits".to_string(),
        "/api/v1/nothing".to_string(),
        format!("/{slug}/api/nothing"),
    ] {
        assert_eq!(get(&app, "GET", &bad, HOST).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
    let unknown = format!("/{slug}/api/audits/audit_{}/detail", "0".repeat(64));
    assert_eq!(get(&app, "GET", &unknown, HOST).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_filters_by_author_and_gap_prefix_and_pages() {
    let (d, _) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    let total = |q: &str| {
        let app = app.clone();
        let uri = format!("/{slug}/api/audits{q}");
        async move { json(&get(&app, "GET", &uri, HOST).await.2)["total"].as_u64().unwrap() }
    };
    assert_eq!(total("").await, 1);
    assert_eq!(total("?author=test-user").await, 1);
    assert_eq!(total("?author=nobody").await, 0);
    assert_eq!(total("?gap=CAPTURE_UNAVAILABLE").await, 1);
    assert_eq!(total("?gap=NO_HUMAN").await, 1);
    assert_eq!(total("?gap=CAPTURE_PARTIAL").await, 0);
    assert_eq!(total("?gap=NO_SUCH.*").await, 0, "the gap filter is a prefix, not a regex");
    let page2 = json(&get(&app, "GET", &format!("/{slug}/api/audits?page=2"), HOST).await.2);
    assert_eq!(page2["audits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn tampering_is_visible_in_the_detail_verification() {
    let (d, id) = fixture();
    let app = app(&d);
    let slug = slug(&d);
    let path = d.path().join(format!(".miaudit/commits/{id}.json"));
    let text = fs::read_to_string(&path).unwrap().replace("staged_paths_only", "staged_paths_onlx");
    fs::write(&path, text).unwrap();
    let (_, _, body) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/detail"), HOST).await;
    assert_eq!(json(&body)["verification"]["integrity"], "fail", "tampering not shown");
}

#[tokio::test]
async fn uninitialized_repo_reports_a_clear_error() {
    let d = tempfile::tempdir().unwrap();
    let (s, _, body) = get(&app(&d), "GET", "/api/v1/repos", HOST).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json(&body)["error"]["message"].as_str().unwrap().contains("not an initialized audit repository"));
}

#[tokio::test]
async fn screenshot_evidence_is_listed_and_only_real_images_are_served() {
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

    let slug = slug(&d);
    let (s, _, body) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/detail"), HOST).await;
    assert_eq!(s, StatusCode::OK);
    let detail = json(&body);
    let item = |eid: &str| detail["evidence"].as_array().unwrap().iter().find(|e| e["id"] == eid).unwrap().clone();
    assert_eq!(item(&e1)["image"], true);
    assert_eq!(item(&e2)["image"], false, "content that is not an image is never flagged as one");
    assert_eq!(item(&e1)["object"]["body"]["environment"], "staging");
    assert_eq!(item(&e1)["object"]["body"]["phase"], "after");

    let req = Request::builder().uri(format!("/{slug}/api/audits/{id}/evidence/{e1}/image")).header("host", HOST).body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert!(res.headers()["content-security-policy"].to_str().unwrap().contains("img-src 'self'"));
    assert_eq!(res.into_body().collect().await.unwrap().to_bytes().to_vec(), png);

    let (s, _, _) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/evidence/{e2}/image"), HOST).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "content that is not an image must never be served");
    let (s, _, _) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/evidence/evd_{}/image", "0".repeat(64)), HOST).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, "GET", &format!("/{slug}/api/audits/{id}/evidence/{e1}/image"), "evil.example").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pull_request_bindings_are_exposed_on_list_and_detail() {
    use std::os::unix::fs::PermissionsExt;
    let (d, id) = fixture();
    let sha = git(d.path(), &["rev-parse", "HEAD"]);
    let gh = tempfile::tempdir().unwrap();
    let script = gh.path().join("gh");
    fs::write(&script, format!("#!/bin/sh\necho '{{\"number\":7,\"url\":\"https://github.com/acme/repo/pull/7\",\"headRefOid\":\"{sha}\",\"state\":\"OPEN\"}}'\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let bin = audit_bin();
    let out = Command::new(&bin).env("PATH", format!("{}:/usr/bin:/bin", gh.path().display())).env("MIAUDIT_GITHUB_USER", "test-user").current_dir(d.path()).args(["--json", "bind", &id, "--pr", "7"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    let app = app(&d);
    let slug = slug(&d);
    let list = json(&get(&app, "GET", &format!("/{slug}/api/audits"), HOST).await.2);
    let pr = list["audits"][0]["git"].as_array().unwrap().iter().find(|g| !g["pull_request"].is_null()).expect("the PR binding is listed");
    assert_eq!((pr["pull_request"]["number"].as_u64(), pr["pull_request"]["web_url"].as_str()), (Some(7), Some("https://github.com/acme/repo/pull/7")));
    assert_eq!(pr["commit"], sha.as_str());
    let detail = json(&get(&app, "GET", &format!("/{slug}/api/audits/{id}/detail"), HOST).await.2);
    let with_pr = detail["bindings"].as_array().unwrap().iter().find(|b| !b["body"]["pull_request"].is_null()).expect("the PR binding is in the detail");
    assert_eq!((with_pr["body"]["pull_request"]["state"].as_str(), with_pr["body"]["pull_request"]["head_commit"].as_str()), (Some("OPEN"), Some(sha.as_str())));
}

#[tokio::test]
async fn a_superseded_decision_points_to_the_audit_that_replaces_it() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let bin = audit_bin();
    let a = bin.to_str().unwrap();
    let j = |args: &[&str]| json(&run(p, a, args));
    j(&["--json", "init", "--no-hooks"]);
    fs::write(p.join("f.txt"), "x").unwrap();
    j(&["--json", "add", "f.txt"]);
    let snap = j(&["--json", "status"])["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    fs::write(p.join("d1.json"), format!(r#"{{"schema_version":"1","question":"keys?","choice":"left for the developer","alternatives":[],"rationale":"r","risks":[],"snapshot_id":"{snap}"}}"#)).unwrap();
    let old = j(&["--json", "decision", "add", "--file", "d1.json"])["result"]["id"].as_str().unwrap().to_string();
    let first = j(&["--json", "commit", "-m", "first"])["result"]["audit_id"].as_str().unwrap().to_string();
    let snap2 = j(&["--json", "status"])["result"]["snapshot"]["id"].as_str().unwrap().to_string();
    fs::write(p.join("d2.json"), format!(r#"{{"schema_version":"1","question":"keys?","choice":"added","alternatives":[],"rationale":"token given","risks":[],"snapshot_id":"{snap2}","supersedes":["{old}"]}}"#)).unwrap();
    let newer = j(&["--json", "decision", "add", "--file", "d2.json"])["result"]["id"].as_str().unwrap().to_string();
    let second = j(&["--json", "commit", "-m", "follow-up with no file changes"])["result"]["audit_id"].as_str().unwrap().to_string();

    let app = app(&d);
    let slug = slug(&d);
    let first_detail = json(&get(&app, "GET", &format!("/{slug}/api/audits/{first}/detail"), HOST).await.2);
    let marker = &first_detail["decisions"][0]["superseded_by"][0];
    assert_eq!((marker["decision_id"].as_str(), marker["audit_id"].as_str()), (Some(newer.as_str()), Some(second.as_str())));
    let second_detail = json(&get(&app, "GET", &format!("/{slug}/api/audits/{second}/detail"), HOST).await.2);
    assert_eq!(second_detail["decisions"][0]["object"]["body"]["supersedes"][0], old.as_str());
    assert_eq!(second_detail["decisions"][0]["superseded_by"].as_array().unwrap().len(), 0);
    assert_eq!(second_detail["manifest"]["snapshot"]["changes"].as_array().unwrap().len(), 0);
}
