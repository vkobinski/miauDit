
use sha2::{Digest, Sha256};

pub const DOMAIN_AUDIT_COMMIT: &str = "auditAI/audit-commit/v1";
pub const DOMAIN_TREE: &str = "auditAI/tree/v1";
pub const DOMAIN_CONTEXT: &str = "auditAI/context/v1";
pub const DOMAIN_DECISION: &str = "auditAI/decision/v1";
pub const DOMAIN_EVIDENCE: &str = "auditAI/evidence/v1";
pub const DOMAIN_BINDING: &str = "auditAI/git-binding/v1";
pub const DOMAIN_EVENT: &str = "auditAI/event/v1";

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn content_id(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

pub fn domain_digest(domain: &str, bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(domain.as_bytes());
    h.update([0u8]);
    h.update(bytes);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sha256_vector() {
        assert_eq!(content_id(b""), "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(content_id(b"abc"), "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn domains_separate_digests() {
        assert_ne!(domain_digest(DOMAIN_CONTEXT, b"x"), domain_digest(DOMAIN_DECISION, b"x"));
        assert_ne!(domain_digest(DOMAIN_CONTEXT, b"x"), sha256_hex(b"x"));
    }
}
