pub mod api;
pub mod data;

use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

pub const PER_PAGE: u64 = 25;

pub struct AppState {
    pub repo: PathBuf,
    pub port: u16,
    pub audit_bin: PathBuf,
}

type Shared = Arc<AppState>;

pub fn local_slug(repo: &std::path::Path) -> String {
    let dir = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let clean: String = dir.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    format!("local/{}", clean.trim_start_matches('.'))
}

pub async fn serve(repo: PathBuf, port: u16, audit_bin: PathBuf, on_ready: impl FnOnce(std::net::SocketAddr)) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    on_ready(addr);
    axum::serve(listener, router(AppState { repo, port: addr.port(), audit_bin }))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/api/v1/config", get(config))
        .route("/api/v1/repos", get(repos))
        .route("/{owner}/{name}/api/repo", get(repo_info))
        .route("/{owner}/{name}/api/audits", get(audits))
        .route("/{owner}/{name}/api/audits/{id}", get(raw_manifest))
        .route("/{owner}/{name}/api/audits/{id}/detail", get(detail))
        .route("/{owner}/{name}/api/audits/{id}/evidence/{eid}/image", get(evidence_image))
        .fallback(spa)
        .method_not_allowed_fallback(|| async { (StatusCode::METHOD_NOT_ALLOWED, "This viewer only accepts GET requests.") })
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn guard(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let allowed = [format!("127.0.0.1:{}", s.port), format!("localhost:{}", s.port)];
    if !allowed.iter().any(|a| a == host) {
        return (StatusCode::FORBIDDEN, "Unexpected Host header.").into_response();
    }
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(miaudit_web::CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn spa(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "This viewer only accepts GET requests.").into_response();
    }
    miaudit_web::serve(uri.path())
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
}

fn open(s: &Shared) -> Result<data::Repo, Box<Response>> {
    data::Repo::open(&s.repo).map_err(|e| Box::new(error(StatusCode::INTERNAL_SERVER_ERROR, "NOT_INITIALIZED", &e)))
}

fn check_slug(s: &Shared, owner: &str, name: &str) -> Result<(), Box<Response>> {
    if format!("{owner}/{name}") == local_slug(&s.repo) { Ok(()) } else { Err(Box::new(error(StatusCode::NOT_FOUND, "NOT_FOUND", "no such repository"))) }
}

async fn config(State(s): State<Shared>) -> Response {
    Json(json!({"mode": "local", "repo": local_slug(&s.repo)})).into_response()
}

async fn repos(State(s): State<Shared>) -> Response {
    let repo = match open(&s) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    Json(json!({"repos": [{
        "slug": local_slug(&s.repo),
        "repo_id": repo.repo_id(),
        "remote_url": null,
        "audits": repo.summaries.len(),
        "gaps": repo.summaries.iter().map(|x| x.manifest.gaps.len()).sum::<usize>(),
        "last_audit_at": repo.summaries.first().map(|x| x.manifest.created.at.clone()),
        "last_push_at": null,
    }]}))
    .into_response()
}

async fn repo_info(State(s): State<Shared>, Path((owner, name)): Path<(String, String)>) -> Response {
    if let Err(e) = check_slug(&s, &owner, &name) {
        return *e;
    }
    let repo = match open(&s) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    let mut counts: std::collections::BTreeMap<String, u64> = Default::default();
    for x in &repo.summaries {
        if let Some(l) = x.manifest.author.as_ref().and_then(|a| a.github_login.clone()) {
            *counts.entry(l).or_default() += 1;
        }
    }
    let mut authors: Vec<(String, u64)> = counts.into_iter().collect();
    authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Json(json!({
        "repo": {"slug": local_slug(&s.repo), "repo_id": repo.repo_id(), "remote_url": null, "last_push_at": null},
        "authors": authors.into_iter().map(|(login, count)| json!({"login": login, "count": count})).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
struct ListQuery {
    page: Option<u64>,
    author: Option<String>,
    gap: Option<String>,
}

async fn audits(State(s): State<Shared>, Path((owner, name)): Path<(String, String)>, Query(q): Query<ListQuery>) -> Response {
    if let Err(e) = check_slug(&s, &owner, &name) {
        return *e;
    }
    let repo = match open(&s) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    let matching: Vec<&data::Summary> = repo
        .summaries
        .iter()
        .filter(|x| q.author.as_deref().filter(|a| !a.is_empty()).is_none_or(|a| x.manifest.author.as_ref().and_then(|m| m.github_login.as_deref()) == Some(a)))
        .filter(|x| q.gap.as_deref().filter(|g| !g.is_empty()).is_none_or(|g| x.manifest.gaps.iter().any(|gap| gap.code.starts_with(g))))
        .collect();
    let page = q.page.unwrap_or(1).max(1);
    let rows: Vec<serde_json::Value> = api::page_of(&matching, page, PER_PAGE)
        .iter()
        .map(|x| {
            let m = &x.manifest;
            let codes: Vec<String> = m.gaps.iter().map(|g| g.code.clone()).collect();
            api::audit_row(
                &api::RowFields {
                    audit_id: &x.id,
                    created_at: &m.created.at,
                    description: &m.description,
                    author: m.author.as_ref().and_then(|a| a.github_login.as_deref()),
                    changes: m.snapshot.changes.len() as i64,
                    gaps: m.gaps.len() as i64,
                    gap_codes: &codes,
                    parents: &m.parents,
                },
                api::git_links(&x.bindings, &x.id),
            )
        })
        .collect();
    Json(json!({"repo": local_slug(&s.repo), "total": matching.len(), "page": page, "per_page": PER_PAGE, "audits": rows})).into_response()
}

async fn raw_manifest(State(s): State<Shared>, Path((owner, name, id)): Path<(String, String, String)>) -> Response {
    if check_slug(&s, &owner, &name).is_err() || !data::valid_audit_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match data::Repo::open(&s.repo).ok().and_then(|r| r.raw_manifest(&id)) {
        Some(bytes) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn detail(State(s): State<Shared>, Path((owner, name, id)): Path<(String, String, String)>) -> Response {
    if check_slug(&s, &owner, &name).is_err() || !data::valid_audit_id(&id) {
        return error(StatusCode::NOT_FOUND, "NOT_FOUND", "no such audit commit");
    }
    let repo = match open(&s) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    match repo.load(&id) {
        Some(view) => {
            let verification = data::verify_with_cli(&s.audit_bin, &s.repo, &id);
            let ids: Vec<String> = view.decisions.iter().map(|d| d.id.clone()).collect();
            Json(api::detail_json(&view, &verification, &repo.superseded_by(&ids))).into_response()
        }
        None => error(StatusCode::NOT_FOUND, "NOT_FOUND", "no such audit commit"),
    }
}

async fn evidence_image(State(s): State<Shared>, Path((owner, name, id, eid)): Path<(String, String, String, String)>) -> Response {
    if check_slug(&s, &owner, &name).is_err() || !data::valid_audit_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match data::Repo::open(&s.repo).ok().and_then(|r| r.evidence_image(&id, &eid)) {
        Some((mime, bytes)) => ([(header::CONTENT_TYPE, mime), (header::CONTENT_DISPOSITION, "inline")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
