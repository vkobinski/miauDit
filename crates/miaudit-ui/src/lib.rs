
pub mod data;
pub mod html;

use axum::{
    Router,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use std::{path::PathBuf, sync::Arc};

pub struct AppState {
    pub repo: PathBuf,
    pub port: u16,
    pub audit_bin: PathBuf,
}

type Shared = Arc<AppState>;

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/", get(index))
        .route("/audit/{id}", get(detail))
        .route("/audit/{id}/evidence/{eid}/image", get(evidence_image))
        .route("/api/audits", get(api_list))
        .route("/api/audits/{id}", get(api_detail))
        .fallback(|| async { page(StatusCode::NOT_FOUND, html::message("Not found", "No such page.")) })
        .method_not_allowed_fallback(|| async { page(StatusCode::METHOD_NOT_ALLOWED, html::message("Read-only", "This viewer only accepts GET requests.")) })
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn guard(State(s): State<Shared>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let allowed = [format!("127.0.0.1:{}", s.port), format!("localhost:{}", s.port)];
    if !allowed.iter().any(|a| a == host) {
        return page(StatusCode::FORBIDDEN, html::message("Forbidden", "Unexpected Host header."));
    }
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn page(status: StatusCode, body: String) -> Response {
    (status, Html(body)).into_response()
}

async fn index(State(s): State<Shared>) -> Response {
    match data::Repo::open(&s.repo) {
        Ok(repo) => page(StatusCode::OK, html::index(&repo, s.port)),
        Err(e) => page(StatusCode::INTERNAL_SERVER_ERROR, html::message("Cannot read audit data", &e)),
    }
}

async fn detail(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    if !data::valid_audit_id(&id) {
        return page(StatusCode::NOT_FOUND, html::message("Not found", "Invalid audit id."));
    }
    let repo = match data::Repo::open(&s.repo) {
        Ok(r) => r,
        Err(e) => return page(StatusCode::INTERNAL_SERVER_ERROR, html::message("Cannot read audit data", &e)),
    };
    match repo.load(&id) {
        Some(view) => {
            let verification = data::verify_with_cli(&s.audit_bin, &s.repo, &id);
            page(StatusCode::OK, html::detail(&view, &verification))
        }
        None => page(StatusCode::NOT_FOUND, html::message("Not found", "No such audit commit.")),
    }
}

async fn evidence_image(State(s): State<Shared>, Path((id, eid)): Path<(String, String)>) -> Response {
    if !data::valid_audit_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match data::Repo::open(&s.repo).ok().and_then(|r| r.evidence_image(&id, &eid)) {
        Some((mime, bytes)) => ([(header::CONTENT_TYPE, mime), (header::CONTENT_DISPOSITION, "inline")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn api_list(State(s): State<Shared>) -> Response {
    match data::Repo::open(&s.repo) {
        Ok(r) => json(r.summaries_json()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn api_detail(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    if !data::valid_audit_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match data::Repo::open(&s.repo).ok().and_then(|r| r.raw_manifest(&id)) {
        Some(bytes) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn json(v: serde_json::Value) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], v.to_string()).into_response()
}
