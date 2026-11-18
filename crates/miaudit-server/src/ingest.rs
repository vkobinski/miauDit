
use miaudit_schema::{
    canonical::to_canonical_bytes,
    digest::{DOMAIN_CONTEXT, DOMAIN_DECISION, DOMAIN_EVIDENCE, content_id, domain_digest},
    input::{ContextInput, DecisionInput},
    model::{EvidenceBody, Manifest, Stored, StoredBinding, Tree, snapshot_id, tree_digest},
};
use serde_json::Value;

pub const MAX_BLOB: usize = 15 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub message: String,
}

fn reject<T>(code: &'static str, message: impl Into<String>) -> Result<T, Rejection> {
    Err(Rejection { code, message: message.into() })
}

pub fn valid_repo_id(id: &str) -> bool {
    id.len() == 5 + 32 && id.starts_with("repo_") && id[5..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn valid_audit_id(id: &str) -> bool {
    id.len() == 6 + 64 && id.starts_with("audit_") && id[6..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn valid_digest(d: &str) -> bool {
    d.strip_prefix("sha256:").is_some_and(hex64)
}

pub fn valid_object_id(id: &str) -> bool {
    ["ctx_", "dec_", "evd_"].iter().any(|p| id.strip_prefix(p).is_some_and(hex64))
}

pub fn valid_binding_id(id: &str) -> bool {
    id.strip_prefix("bind_").is_some_and(hex64)
}

pub fn valid_slug(slug: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty() && p.len() <= 100 && p != "." && p != ".." && !p.starts_with('.') && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    match slug.split_once('/') {
        Some((o, n)) => ok(o) && ok(n) && !n.contains('/') && !["api", "static", "assets"].contains(&o),
        None => false,
    }
}

pub fn check_blob(digest: &str, bytes: &[u8]) -> Result<(), Rejection> {
    if !valid_digest(digest) {
        return reject("BAD_DIGEST", "digest must be sha256:<64 hex>");
    }
    if bytes.len() > MAX_BLOB {
        return reject("BLOB_TOO_LARGE", format!("blob is {} bytes; the limit is {MAX_BLOB}", bytes.len()));
    }
    if content_id(bytes) != digest {
        return reject("DIGEST_MISMATCH", "body does not hash to the digest in the URL");
    }
    Ok(())
}

pub enum Object {
    Context,
    Decision,
    Evidence(EvidenceBody),
}

pub fn check_object(id: &str, bytes: &[u8]) -> Result<Object, Rejection> {
    if !valid_object_id(id) {
        return reject("BAD_ID", "object id must be ctx_/dec_/evd_ followed by 64 hex");
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| Rejection { code: "BAD_JSON", message: e.to_string() })?;
    let canon = to_canonical_bytes(&value).map_err(|e| Rejection { code: "NOT_CANONICAL", message: e.to_string() })?;
    let (prefix, domain) = match &id[..3] {
        "ctx" => ("ctx", DOMAIN_CONTEXT),
        "dec" => ("dec", DOMAIN_DECISION),
        _ => ("evd", DOMAIN_EVIDENCE),
    };
    if format!("{prefix}_{}", domain_digest(domain, &canon)) != id {
        return reject("ID_MISMATCH", "content does not hash to the object id");
    }
    let parse_err = |e: serde_json::Error| Rejection { code: "SCHEMA_VIOLATION", message: e.to_string() };
    match prefix {
        "ctx" => serde_json::from_value::<Stored<ContextInput>>(value).map(|_| Object::Context).map_err(parse_err),
        "dec" => serde_json::from_value::<Stored<DecisionInput>>(value).map(|_| Object::Decision).map_err(parse_err),
        _ => serde_json::from_value::<Stored<EvidenceBody>>(value).map(|s| Object::Evidence(s.body)).map_err(parse_err),
    }
}

pub fn check_commit(repo_id: &str, id: &str, bytes: &[u8]) -> Result<Manifest, Rejection> {
    if !valid_audit_id(id) {
        return reject("BAD_ID", "audit id must be audit_ followed by 64 hex");
    }
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| Rejection { code: "SCHEMA_VIOLATION", message: e.to_string() })?;
    if m.repo_id != repo_id {
        return reject("REPO_MISMATCH", format!("manifest belongs to {}, not {repo_id}", m.repo_id));
    }
    match m.audit_id() {
        Ok(got) if got == id => {}
        Ok(got) => return reject("ID_MISMATCH", format!("manifest hashes to {got}")),
        Err(e) => return reject("NOT_CANONICAL", e.to_string()),
    }
    let tree: Tree = m.snapshot.entries.iter().map(|e| (e.path.clone(), e.clone())).collect();
    match tree_digest(&tree) {
        Ok(td) if td == m.snapshot.tree_digest && snapshot_id(&td) == m.snapshot.id && tree.len() == m.snapshot.entries.len() => Ok(m),
        _ => reject("TREE_MISMATCH", "snapshot entries do not match tree_digest / snapshot id"),
    }
}

pub fn check_binding(id: &str, bytes: &[u8]) -> Result<StoredBinding, Rejection> {
    if !valid_binding_id(id) {
        return reject("BAD_ID", "binding id must be bind_ followed by 64 hex");
    }
    let b: StoredBinding = serde_json::from_slice(bytes).map_err(|e| Rejection { code: "SCHEMA_VIOLATION", message: e.to_string() })?;
    if b.binding_id != id || b.body.binding_id().ok().as_deref() != Some(id) {
        return reject("ID_MISMATCH", "binding does not hash to its id");
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rules() {
        assert!(valid_slug("acme/web"));
        assert!(valid_slug("local/my_repo.v2"));
        for bad in ["", "acme", "acme/", "/web", "a/b/c", "api/x", "acme/..", ".hidden/x", "acme/we b", "acme/we<b>"] {
            assert!(!valid_slug(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn blob_must_hash_to_digest_and_fit() {
        let d = content_id(b"hello");
        assert_eq!(check_blob(&d, b"hello"), Ok(()));
        assert_eq!(check_blob(&d, b"hellO").unwrap_err().code, "DIGEST_MISMATCH");
        assert_eq!(check_blob("sha256:zz", b"x").unwrap_err().code, "BAD_DIGEST");
        let big = vec![0u8; MAX_BLOB + 1];
        assert_eq!(check_blob(&content_id(&big), &big).unwrap_err().code, "BLOB_TOO_LARGE");
    }

    #[test]
    fn object_must_hash_to_its_id() {
        let ctx = br#"{"kind":"context","schema_version":"1","provenance":"self_reported","body":{"schema_version":"1","description":"d","task_ref":null,"constraints":[],"source_refs":[]}}"#;
        let v: Value = serde_json::from_slice(ctx).unwrap();
        let id = format!("ctx_{}", domain_digest(DOMAIN_CONTEXT, &to_canonical_bytes(&v).unwrap()));
        assert!(matches!(check_object(&id, ctx), Ok(Object::Context)));
        let other = format!("ctx_{}", "0".repeat(64));
        assert_eq!(check_object(&other, ctx).err().unwrap().code, "ID_MISMATCH");
        let as_dec = format!("dec_{}", domain_digest(DOMAIN_DECISION, &to_canonical_bytes(&v).unwrap()));
        assert_eq!(check_object(&as_dec, ctx).err().unwrap().code, "SCHEMA_VIOLATION");
    }

    #[test]
    fn id_shapes() {
        assert!(valid_repo_id(&format!("repo_{}", "a".repeat(32))));
        assert!(!valid_repo_id("repo_../../x"));
        assert!(!valid_audit_id("audit_xyz"));
        assert!(!valid_object_id("ctx_short"));
    }
}
