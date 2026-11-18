use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Assets;

pub const STAMP: &str = env!("MIAUDIT_WEB_STAMP");

pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

pub fn is_api_path(path: &str) -> bool {
    path.starts_with("/api/") || path.contains("/api/")
}

pub fn serve(path: &str) -> Response {
    let clean = path.trim_start_matches('/');
    if let Some(file) = Assets::get(clean).filter(|_| !clean.is_empty()) {
        return asset(file.metadata.mimetype(), file.data.into_owned());
    }
    if is_api_path(path) || clean.contains('.') && clean.starts_with("assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get("index.html") {
        Some(index) => asset("text/html; charset=utf-8", index.data.into_owned()),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "web UI missing").into_response(),
    }
}

fn asset(mime: &str, bytes: Vec<u8>) -> Response {
    let mut res = (StatusCode::OK, bytes).into_response();
    if let Ok(v) = HeaderValue::from_str(mime) {
        res.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spa_routes_get_the_shell_and_api_paths_do_not() {
        for p in ["/", "/acme/web", "/acme/web/audit/audit_x"] {
            let r = serve(p);
            assert_eq!(r.status(), StatusCode::OK, "{p}");
            assert!(r.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"), "{p}");
        }
        for p in ["/api/v1/nothing", "/acme/web/api/nothing", "/assets/missing.js"] {
            assert_eq!(serve(p).status(), StatusCode::NOT_FOUND, "{p}");
        }
    }

    #[test]
    fn the_built_bundle_is_embedded_with_correct_types() {
        let index = String::from_utf8(Assets::get("index.html").expect("index").data.into_owned()).unwrap();
        assert!(!index.contains("<script>"), "no inline script is allowed by the CSP");
        let js = index.split("src=\"/assets/").nth(1).and_then(|s| s.split('"').next()).expect("script tag");
        let r = serve(&format!("/assets/{js}"));
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers()[header::CONTENT_TYPE].to_str().unwrap().contains("javascript"));
    }

    #[test]
    fn the_pixel_icon_is_embedded_linked_from_the_shell_and_served_as_a_png() {
        let index = String::from_utf8(Assets::get("index.html").expect("index").data.into_owned()).unwrap();
        assert!(index.contains(r#"rel="icon""#) && index.contains("/icon-64.png"), "the shell links the favicon: {index}");
        let r = serve("/icon-64.png");
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/png");
        let bytes = Assets::get("icon-64.png").expect("icon embedded").data.into_owned();
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!((u32::from_be_bytes(bytes[16..20].try_into().unwrap()), u32::from_be_bytes(bytes[20..24].try_into().unwrap())), (64, 64));
        assert_eq!(serve("/missing-icon.png").status(), StatusCode::OK, "an unknown path still gets the app shell, not a broken image");
    }
}
