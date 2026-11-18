pub mod db;
pub mod ingest;
pub mod web;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use base64::Engine;
use db::{Db, DbError};
use miaudit_ui::data::Verification;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub struct AppState {
    pub db: Db,
    pub read_password: Option<String>,
}

type Shared = Arc<AppState>;

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);
    let push = Router::new()
        .route("/api/v1/repos/{repo_id}", put(register))
        .route("/api/v1/repos/{repo_id}/have", post(have))
        .route("/api/v1/repos/{repo_id}/blobs/{digest}", put(put_blob))
        .route("/api/v1/repos/{repo_id}/objects/{id}", put(put_object))
        .route("/api/v1/repos/{repo_id}/commits/{id}", put(put_commit))
        .route("/api/v1/repos/{repo_id}/bindings/{id}", put(put_binding))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));
    let read = Router::new()
        .route("/", get(home))
        .route("/api/v1/repos", get(api_repos))
        .route("/{owner}/{name}", get(repo_page))
        .route("/{owner}/{name}/audit/{id}", get(audit_page))
        .route("/{owner}/{name}/audit/{id}/evidence/{eid}/image", get(evidence_image))
        .route("/{owner}/{name}/api/audits", get(api_audits))
        .route("/{owner}/{name}/api/audits/{id}", get(api_audit))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_read));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(push)
        .merge(read)
        .fallback(|| async { html(StatusCode::NOT_FOUND, web::message("Not found", "No such page.")) })
        .layer(middleware::from_fn(secure_headers))
        .with_state(state)
}

async fn secure_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_read(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let Some(expected) = &s.read_password else { return next.run(req).await };
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|v| v.split_once(':').map(|(_, p)| p.to_string()));
    if supplied.is_some_and(|p| ct_eq(p.as_bytes(), expected.as_bytes())) {
        return next.run(req).await;
    }
    (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, r#"Basic realm="miauDit""#)], "authentication required").into_response()
}

async fn require_token(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let token = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_string);
    match token {
        Some(t) => match s.db.token_valid(&t).await {
            Ok(true) => next.run(req).await,
            Ok(false) => api_error(StatusCode::UNAUTHORIZED, "BAD_TOKEN", "unknown or revoked token"),
            Err(e) => db_error(e),
        },
        None => api_error(StatusCode::UNAUTHORIZED, "NO_TOKEN", "send Authorization: Bearer <token>"),
    }
}

fn html(status: StatusCode, body: String) -> Response {
    (status, Html(body)).into_response()
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
}

fn db_error(e: DbError) -> Response {
    match e {
        DbError::Rejected(r) => {
            let status = if r.code == "BLOB_TOO_LARGE" { StatusCode::PAYLOAD_TOO_LARGE } else { StatusCode::UNPROCESSABLE_ENTITY };
            api_error(status, r.code, &r.message)
        }
        DbError::Conflict(m) => api_error(StatusCode::CONFLICT, "CONFLICT", &m),
        DbError::NotFound(m) => api_error(StatusCode::NOT_FOUND, "NOT_FOUND", &m),
        DbError::Mongo(e) => {
            eprintln!("miaudit-server: database error: {e}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE", "database error")
        }
    }
}

fn page_error(e: DbError) -> Response {
    eprintln!("miaudit-server: {e:?}");
    html(StatusCode::INTERNAL_SERVER_ERROR, web::message("Database error", "The audit database could not be read."))
}

#[derive(Deserialize)]
struct Register {
    slug: String,
    remote_url: Option<String>,
}

async fn register(State(s): State<Shared>, Path(repo_id): Path<String>, Json(b): Json<Register>) -> Response {
    match s.db.register_repo(&repo_id, &b.slug, b.remote_url.as_deref()).await {
        Ok(r) => Json(json!({"repo_id": r.repo_id, "slug": r.slug})).into_response(),
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Have {
    commits: Vec<String>,
    objects: Vec<String>,
    blobs: Vec<String>,
    bindings: Vec<String>,
}

async fn have(State(s): State<Shared>, Path(repo_id): Path<String>, Json(b): Json<Have>) -> Response {
    if b.commits.len() + b.objects.len() + b.blobs.len() + b.bindings.len() > 50_000 {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "TOO_MANY", "at most 50000 ids per request");
    }
    match s.db.missing(&repo_id, &b.commits, &b.objects, &b.blobs, &b.bindings).await {
        Ok([commits, objects, blobs, bindings]) => Json(json!({"missing": {"commits": commits, "objects": objects, "blobs": blobs, "bindings": bindings}})).into_response(),
        Err(e) => db_error(e),
    }
}

fn stored(r: Result<bool, DbError>) -> Response {
    match r {
        Ok(new) => Json(json!({"stored": new})).into_response(),
        Err(e) => db_error(e),
    }
}

async fn put_blob(State(s): State<Shared>, Path((repo_id, digest)): Path<(String, String)>, body: Bytes) -> Response {
    stored(s.db.put_blob(&repo_id, &digest, &body).await)
}
async fn put_object(State(s): State<Shared>, Path((repo_id, id)): Path<(String, String)>, body: Bytes) -> Response {
    stored(s.db.put_object(&repo_id, &id, &body).await)
}
async fn put_commit(State(s): State<Shared>, Path((repo_id, id)): Path<(String, String)>, body: Bytes) -> Response {
    stored(s.db.put_commit(&repo_id, &id, &body).await)
}
async fn put_binding(State(s): State<Shared>, Path((repo_id, id)): Path<(String, String)>, body: Bytes) -> Response {
    stored(s.db.put_binding(&repo_id, &id, &body).await)
}

#[derive(Deserialize, Default)]
struct HomeQuery {
    q: Option<String>,
}

async fn home(State(s): State<Shared>, Query(q): Query<HomeQuery>) -> Response {
    let term = q.q.unwrap_or_default();
    match s.db.repo_stats(Some(&term)).await {
        Ok(stats) => html(StatusCode::OK, web::home(&stats, &term)),
        Err(e) => page_error(e),
    }
}

async fn api_repos(State(s): State<Shared>) -> Response {
    match s.db.repo_stats(None).await {
        Ok(stats) => Json(json!({"repos": stats.iter().map(|r| json!({
            "slug": r.repo.slug, "repo_id": r.repo.repo_id, "remote_url": r.repo.remote_url,
            "audits": r.audits, "gaps": r.gaps, "last_audit_at": r.last_audit_at, "last_push_at": r.repo.last_push_at,
        })).collect::<Vec<_>>()}))
        .into_response(),
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize, Default)]
struct RepoQuery {
    page: Option<u64>,
    author: Option<String>,
    gap: Option<String>,
}

async fn find_repo(s: &Shared, owner: &str, name: &str) -> Result<db::RepoDoc, Response> {
    let slug = format!("{owner}/{name}");
    if !ingest::valid_slug(&slug) {
        return Err(html(StatusCode::NOT_FOUND, web::message("Not found", "No such repository.")));
    }
    match s.db.repo_by_slug(&slug).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(html(StatusCode::NOT_FOUND, web::message("Not found", "No such repository."))),
        Err(e) => Err(page_error(e)),
    }
}

async fn repo_page(State(s): State<Shared>, Path((owner, name)): Path<(String, String)>, Query(q): Query<RepoQuery>) -> Response {
    let repo = match find_repo(&s, &owner, &name).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let page = q.page.unwrap_or(1).max(1);
    let result = async {
        let (audits, total) = s.db.audit_page(&repo.repo_id, q.author.as_deref(), None, page).await?;
        let ids: Vec<String> = audits.iter().map(|a| a.audit_id.clone()).collect();
        let bindings = s.db.bindings_for(&repo.repo_id, &ids).await?;
        let authors = s.db.authors(&repo.repo_id).await?;
        Ok::<_, DbError>((audits, total, bindings, authors))
    }
    .await;
    match result {
        Ok((audits, total, bindings, authors)) => html(
            StatusCode::OK,
            web::repo(&web::RepoPage { repo: &repo, audits: &audits, bindings: &bindings, total, page, author: q.author.as_deref(), authors: &authors }),
        ),
        Err(e) => page_error(e),
    }
}

async fn audit_page(State(s): State<Shared>, Path((owner, name, id)): Path<(String, String, String)>) -> Response {
    let repo = match find_repo(&s, &owner, &name).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !ingest::valid_audit_id(&id) {
        return html(StatusCode::NOT_FOUND, web::message("Not found", "Invalid audit id."));
    }
    match s.db.load_view(&repo.repo_id, &id).await {
        Ok(Some((view, checks))) => {
            let integrity = if checks.iter().any(|c| c.1 == "fail") { "fail" } else if checks.iter().any(|c| c.1 == "partial") { "partial" } else { "pass" };
            let ver = Verification::Result { integrity: integrity.into(), checks };
            html(StatusCode::OK, web::audit(&repo, &view, &ver))
        }
        Ok(None) => html(StatusCode::NOT_FOUND, web::message("Not found", "No such audit commit in this repository.")),
        Err(e) => page_error(e),
    }
}

async fn evidence_image(State(s): State<Shared>, Path((owner, name, id, eid)): Path<(String, String, String, String)>) -> Response {
    let Ok(repo) = find_repo(&s, &owner, &name).await else { return StatusCode::NOT_FOUND.into_response() };
    if !ingest::valid_audit_id(&id) || !ingest::valid_object_id(&eid) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match s.db.evidence_image(&repo.repo_id, &id, &eid).await {
        Ok(Some((mime, bytes))) => ([(header::CONTENT_TYPE, mime), (header::CONTENT_DISPOSITION, "inline")], bytes).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => db_error(e),
    }
}

async fn api_audits(State(s): State<Shared>, Path((owner, name)): Path<(String, String)>, Query(q): Query<RepoQuery>) -> Response {
    let repo = match find_repo(&s, &owner, &name).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let page = q.page.unwrap_or(1).max(1);
    let result = async {
        let (audits, total) = s.db.audit_page(&repo.repo_id, q.author.as_deref(), q.gap.as_deref(), page).await?;
        let ids: Vec<String> = audits.iter().map(|a| a.audit_id.clone()).collect();
        let bindings = s.db.bindings_for(&repo.repo_id, &ids).await?;
        Ok::<_, DbError>((audits, total, bindings))
    }
    .await;
    match result {
        Ok((audits, total, bindings)) => Json(json!({"repo": repo.slug, "total": total, "page": page, "per_page": db::PAGE_SIZE, "audits": audits.iter().map(|a| json!({
            "audit_id": a.audit_id, "created_at": a.created_at, "description": a.description, "author": a.author_login,
            "changes": a.changes, "gaps": a.gaps, "gap_codes": a.gap_codes, "parents": a.parents,
            "git": bindings.iter().filter(|b| b.body.audit_id == a.audit_id).map(|b| json!({"commit": b.body.git.commit, "correspondence": b.body.correspondence})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()}))
        .into_response(),
        Err(e) => db_error(e),
    }
}

async fn api_audit(State(s): State<Shared>, Path((owner, name, id)): Path<(String, String, String)>) -> Response {
    let repo = match find_repo(&s, &owner, &name).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !ingest::valid_audit_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match s.db.raw_commit(&repo.repo_id, &id).await {
        Ok(Some(raw)) => ([(header::CONTENT_TYPE, "application/json")], raw).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => db_error(e),
    }
}
