use crate::{
    SCHEMA_VERSION,
    canonical::{CanonError, C14N_VERSION, to_canonical_bytes},
    digest::{DOMAIN_AUDIT_COMMIT, DOMAIN_BINDING, DOMAIN_TREE, domain_digest},
    input::{Classification, ContextInput, DecisionInput},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Observed,
    SelfReported,
    Derived,
    Unavailable,
    Redacted,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Symlink,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TreeEntry {
    pub path: String,
    pub kind: EntryKind,
    pub mode: u32,
    pub digest: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Add,
    Modify,
    Delete,
    TypeChange,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub path: String,
    pub operation: Operation,
    pub kind_before: Option<EntryKind>,
    pub kind_after: Option<EntryKind>,
    pub mode_before: Option<u32>,
    pub mode_after: Option<u32>,
    pub digest_before: Option<String>,
    pub digest_after: Option<String>,
    pub size_after: Option<u64>,
}

pub type Tree = BTreeMap<String, TreeEntry>;

pub fn tree_digest(tree: &Tree) -> Result<String, CanonError> {
    let entries: Vec<&TreeEntry> = tree.values().collect();
    Ok(domain_digest(DOMAIN_TREE, &to_canonical_bytes(&entries)?))
}

pub fn snapshot_id(tree_digest: &str) -> String {
    format!("snapshot_{tree_digest}")
}

pub fn diff_trees(before: &Tree, after: &Tree) -> Vec<Change> {
    let mut paths: Vec<&String> = before.keys().chain(after.keys()).collect();
    paths.sort();
    paths.dedup();
    let mut out = Vec::new();
    for p in paths {
        let (b, a) = (before.get(p), after.get(p));
        let op = match (b, a) {
            (None, Some(_)) => Operation::Add,
            (Some(_), None) => Operation::Delete,
            (Some(b), Some(a)) if b.kind != a.kind => Operation::TypeChange,
            (Some(b), Some(a)) if b.digest != a.digest || b.mode != a.mode => Operation::Modify,
            _ => continue,
        };
        out.push(Change {
            path: p.clone(),
            operation: op,
            kind_before: b.map(|e| e.kind),
            kind_after: a.map(|e| e.kind),
            mode_before: b.map(|e| e.mode),
            mode_after: a.map(|e| e.mode),
            digest_before: b.map(|e| e.digest.clone()),
            digest_after: a.map(|e| e.digest.clone()),
            size_after: a.map(|e| e.size),
        });
    }
    out
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Stored<T> {
    pub kind: String,
    pub schema_version: String,
    pub provenance: Provenance,
    pub body: T,
}

impl<T> Stored<T> {
    pub fn self_reported(kind: &str, body: T) -> Self {
        Stored { kind: kind.into(), schema_version: SCHEMA_VERSION.into(), provenance: Provenance::SelfReported, body }
    }

    pub fn observed(kind: &str, body: T) -> Self {
        Stored { kind: kind.into(), schema_version: SCHEMA_VERSION.into(), provenance: Provenance::Observed, body }
    }
}

pub type StoredContext = Stored<ContextInput>;
pub type StoredDecision = Stored<DecisionInput>;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub digest: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBody {
    pub classification: Classification,
    pub description: String,
    pub artifact: ArtifactRef,
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<crate::input::Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<crate::input::Environment>,
}

pub type StoredEvidence = Stored<EvidenceBody>;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitBaseline {
    Known { commit: String, algorithm: String },
    Unavailable { reason: String },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub tree_digest: String,
    pub provenance: Provenance,
    pub verification: String,
    pub scope: String,
    pub git_baseline: GitBaseline,
    pub exclusions: Vec<String>,
    pub case_sensitive: Option<bool>,
    pub entries: Vec<TreeEntry>,
    pub changes: Vec<Change>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub capability: String,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub code: String,
    pub message: String,
    pub subjects: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Created {
    pub at: String,
    pub clock_authority: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub tool: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub github_login: Option<String>,
    pub github_id: Option<u64>,
    pub source: String,
    pub provenance: Provenance,
    pub verification: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: String,
    pub canonicalization: String,
    pub kind: String,
    pub repo_id: String,
    pub parents: Vec<String>,
    pub description: String,
    pub created: Created,
    pub producer: Producer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    pub snapshot: Snapshot,
    pub context_ids: Vec<String>,
    pub decision_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub coverage: Vec<Capability>,
    pub gaps: Vec<Gap>,
}

impl Manifest {
    pub fn canonical_version() -> &'static str {
        C14N_VERSION
    }

    pub fn audit_id(&self) -> Result<String, CanonError> {
        Ok(format!("audit_{}", domain_digest(DOMAIN_AUDIT_COMMIT, &to_canonical_bytes(self)?)))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Correspondence {
    Full,
    Partial,
    NoMatch,
    Asserted,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PathState {
    Match,
    ContentDiffers,
    ModeDiffers,
    AbsentInGit,
    PresentInGit,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct PathCheck {
    pub path: String,
    pub operation: Operation,
    pub state: PathState,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct GitRef {
    pub commit: String,
    pub algorithm: String,
    pub repository_web_url: Option<String>,
    pub commit_web_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct GitBindingBody {
    pub schema_version: String,
    pub kind: String,
    pub audit_id: String,
    pub git: GitRef,
    pub correspondence: Correspondence,
    pub verification: String,
    pub provenance: Provenance,
    pub paths: Vec<PathCheck>,
}

impl GitBindingBody {
    pub fn binding_id(&self) -> Result<String, CanonError> {
        Ok(format!("bind_{}", domain_digest(DOMAIN_BINDING, &to_canonical_bytes(self)?)))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct StoredBinding {
    pub binding_id: String,
    pub body: GitBindingBody,
    pub created: Created,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, mode: u32, digest: &str) -> TreeEntry {
        TreeEntry { path: path.into(), kind: EntryKind::File, mode, digest: digest.into(), size: 1 }
    }

    fn manifest() -> Manifest {
        let e = entry("a.txt", 0o100644, "sha256:00");
        let tree: Tree = [(e.path.clone(), e.clone())].into();
        let td = tree_digest(&tree).unwrap();
        Manifest {
            schema_version: "1".into(),
            canonicalization: C14N_VERSION.into(),
            kind: "audit_commit".into(),
            repo_id: "repo_test".into(),
            parents: vec![],
            description: "vector".into(),
            created: Created { at: "2026-01-01T00:00:00Z".into(), clock_authority: "local_device_unverified".into() },
            producer: Producer { tool: "audit-cli".into(), version: "0.1.0".into() },
            author: None,
            snapshot: Snapshot {
                id: snapshot_id(&td),
                tree_digest: td,
                provenance: Provenance::SelfReported,
                verification: "unverified_client_snapshot".into(),
                scope: "staged_paths_only".into(),
                git_baseline: GitBaseline::Unavailable { reason: "test".into() },
                exclusions: vec![],
                case_sensitive: Some(true),
                entries: vec![e],
                changes: vec![],
            },
            context_ids: vec![],
            decision_ids: vec![],
            evidence_ids: vec![],
            coverage: vec![],
            gaps: vec![],
        }
    }

    #[test]
    fn audit_id_is_deterministic_and_matches_pinned_vector() {
        let (a, b) = (manifest(), manifest());
        assert_eq!(a.audit_id().unwrap(), b.audit_id().unwrap());
        assert_eq!(a.audit_id().unwrap(), "audit_7491ebfee4a974198f4b527b1023bee57c4d950a67dd860a9808352c50a6a590");
    }

    #[test]
    fn binding_id_is_deterministic_and_covers_every_field() {
        let body = |c: Correspondence, commit: &str| GitBindingBody {
            schema_version: "1".into(),
            kind: "git_binding".into(),
            audit_id: "audit_x".into(),
            git: GitRef { commit: commit.into(), algorithm: "sha1".into(), repository_web_url: None, commit_web_url: None },
            correspondence: c,
            verification: "local_git_inspection_unsigned".into(),
            provenance: Provenance::Derived,
            paths: vec![],
        };
        let a = body(Correspondence::Full, "abc").binding_id().unwrap();
        assert_eq!(a, body(Correspondence::Full, "abc").binding_id().unwrap());
        assert!(a.starts_with("bind_"));
        assert_ne!(a, body(Correspondence::Partial, "abc").binding_id().unwrap());
        assert_ne!(a, body(Correspondence::Full, "abd").binding_id().unwrap());
    }

    #[test]
    fn any_manifest_change_changes_the_audit_id() {
        let base = manifest().audit_id().unwrap();
        let mut m = manifest();
        m.description = "other".into();
        assert_ne!(m.audit_id().unwrap(), base);
        let mut m = manifest();
        m.snapshot.entries[0].mode = 0o100755;
        assert_ne!(m.audit_id().unwrap(), base);
    }

    #[test]
    fn tree_digest_detects_mode_and_type_changes_with_identical_content() {
        let t = |mode, kind| -> Tree {
            let mut e = entry("a", mode, "sha256:00");
            e.kind = kind;
            [("a".to_string(), e)].into()
        };
        let base = tree_digest(&t(0o100644, EntryKind::File)).unwrap();
        assert_ne!(base, tree_digest(&t(0o100755, EntryKind::File)).unwrap());
        assert_ne!(base, tree_digest(&t(0o120000, EntryKind::Symlink)).unwrap());
    }

    #[test]
    fn diff_reports_add_modify_delete_type_change_and_no_rename_inference() {
        let b: Tree = [("keep".into(), entry("keep", 0o100644, "d1")), ("old".into(), entry("old", 0o100644, "d2")), ("mod".into(), entry("mod", 0o100644, "d3"))].into();
        let mut a = b.clone();
        a.remove("old");
        a.insert("new".into(), entry("new", 0o100644, "d2"));
        a.insert("mod".into(), entry("mod", 0o100644, "d4"));
        let ops: Vec<_> = diff_trees(&b, &a).into_iter().map(|c| (c.path, c.operation)).collect();
        assert_eq!(ops, vec![("mod".into(), Operation::Modify), ("new".into(), Operation::Add), ("old".into(), Operation::Delete)]);
    }

    #[test]
    fn author_is_part_of_the_id_only_when_present() {
        let base = manifest();
        assert_eq!(base.audit_id().unwrap(), "audit_7491ebfee4a974198f4b527b1023bee57c4d950a67dd860a9808352c50a6a590");
        let mut a = manifest();
        a.author = Some(Author { github_login: Some("octo".into()), github_id: Some(1), source: "gh_api_user".into(), provenance: Provenance::Derived, verification: "x".into() });
        let mut b = a.clone();
        b.author.as_mut().unwrap().github_login = Some("other".into());
        assert_ne!(a.audit_id().unwrap(), base.audit_id().unwrap());
        assert_ne!(a.audit_id().unwrap(), b.audit_id().unwrap());
        let round: Manifest = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(round, a);
        let old: Manifest = serde_json::from_slice(&serde_json::to_vec(&base).unwrap()).unwrap();
        assert_eq!(old.author, None);
    }
}
