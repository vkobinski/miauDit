
use miaudit_schema::{
    input::{ContextInput, DecisionInput},
    model::{EvidenceBody, Manifest, Stored, StoredBinding},
};
use serde_json::{Value, json};
use std::{fs, path::{Path, PathBuf}, process::Command};

pub fn valid_audit_id(id: &str) -> bool {
    id.len() == 6 + 64 && id.starts_with("audit_") && id[6..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn safe_object_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub struct Summary {
    pub id: String,
    pub manifest: Manifest,
    pub bindings: Vec<StoredBinding>,
}

pub struct Repo {
    pub dir: PathBuf,
    pub root: PathBuf,
    pub summaries: Vec<Summary>,
    pub unreadable: usize,
}

pub struct Item<T> {
    pub id: String,
    pub object: Option<Stored<T>>,
}

pub struct EvidenceView {
    pub id: String,
    pub object: Option<Stored<EvidenceBody>>,
    pub preview: Option<String>,
    pub artifact_state: &'static str,
}

pub struct View {
    pub id: String,
    pub manifest: Manifest,
    pub bindings: Vec<StoredBinding>,
    pub contexts: Vec<Item<ContextInput>>,
    pub decisions: Vec<Item<DecisionInput>>,
    pub evidence: Vec<EvidenceView>,
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(p).ok()?).ok()
}

impl Repo {
    pub fn open(root: &Path) -> Result<Repo, String> {
        let dir = root.join(".audit");
        if !dir.join("config.json").is_file() {
            return Err(format!("{} is not an initialized audit repository (no .audit/config.json)", root.display()));
        }
        let bindings: Vec<StoredBinding> = fs::read_dir(dir.join("bindings")).into_iter().flatten().flatten()
            .filter_map(|e| read_json(&e.path())).collect();
        let (mut summaries, mut unreadable) = (Vec::new(), 0);
        for e in fs::read_dir(dir.join("commits")).map_err(|e| e.to_string())?.flatten() {
            let path = e.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else { continue };
            if !valid_audit_id(&id) {
                continue;
            }
            match read_json::<Manifest>(&path) {
                Some(manifest) => {
                    let b = bindings.iter().filter(|b| b.body.audit_id == id).cloned().collect();
                    summaries.push(Summary { id, manifest, bindings: b });
                }
                None => unreadable += 1,
            }
        }
        summaries.sort_by(|a, b| b.manifest.created.at.cmp(&a.manifest.created.at).then(a.id.cmp(&b.id)));
        Ok(Repo { dir, root: root.to_path_buf(), summaries, unreadable })
    }

    pub fn raw_manifest(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.dir.join("commits").join(format!("{id}.json"))).ok()
    }

    fn object<T: serde::de::DeserializeOwned>(&self, sub: &str, id: &str) -> Option<Stored<T>> {
        if !safe_object_id(id) {
            return None;
        }
        read_json(&self.dir.join("objects").join(sub).join(format!("{id}.json")))
    }

    pub fn load(&self, id: &str) -> Option<View> {
        let s = self.summaries.iter().find(|s| s.id == id)?;
        let m = &s.manifest;
        let evidence = m.evidence_ids.iter().map(|eid| {
            let object: Option<Stored<EvidenceBody>> = self.object("evidence", eid);
            let (mut preview, mut state) = (None, "object unavailable");
            if let Some(o) = &object {
                let hex = o.body.artifact.digest.strip_prefix("sha256:").unwrap_or("");
                state = "artifact unavailable";
                if hex.len() == 64
                    && hex.bytes().all(|b| b.is_ascii_hexdigit())
                    && let Ok(bytes) = fs::read(self.dir.join("blobs").join(hex))
                {
                    state = "present (digest checked by verify)";
                    if bytes.len() <= 8192 {
                        preview = String::from_utf8(bytes).ok();
                    } else {
                        state = "present, too large to preview";
                    }
                }
            }
            EvidenceView { id: eid.clone(), object, preview, artifact_state: state }
        }).collect();
        Some(View {
            id: id.into(),
            manifest: m.clone(),
            bindings: s.bindings.clone(),
            contexts: m.context_ids.iter().map(|i| Item { id: i.clone(), object: self.object("contexts", i) }).collect(),
            decisions: m.decision_ids.iter().map(|i| Item { id: i.clone(), object: self.object("decisions", i) }).collect(),
            evidence,
        })
    }

    pub fn summaries_json(&self) -> Value {
        json!({"repo": self.root, "unreadable": self.unreadable, "audits": self.summaries.iter().map(|s| json!({
            "audit_id": s.id, "created_at": s.manifest.created.at, "description": s.manifest.description,
            "changes": s.manifest.snapshot.changes.len(), "gaps": s.manifest.gaps.len(), "parents": s.manifest.parents,
            "git": s.bindings.iter().map(|b| json!({"commit": b.body.git.commit, "correspondence": b.body.correspondence, "url": b.body.git.commit_web_url})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>()})
    }
}

pub enum Verification {
    Unavailable(String),
    Result { integrity: String, checks: Vec<(String, String, String)> },
}

pub fn verify_with_cli(bin: &Path, repo: &Path, id: &str) -> Verification {
    let out = match Command::new(bin).args(["--json", "verify", id]).current_dir(repo).output() {
        Ok(o) => o,
        Err(e) => return Verification::Unavailable(format!("could not run {}: {e}", bin.display())),
    };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Verification::Unavailable("verify produced no JSON output".into());
    };
    let Some(r) = v.get("result") else {
        return Verification::Unavailable(v["error"]["rule"].as_str().unwrap_or("verify failed").to_string());
    };
    let s = |x: &Value| x.as_str().unwrap_or("?").to_string();
    Verification::Result {
        integrity: s(&r["verified"]["integrity"]),
        checks: r["checks"].as_array().map(|a| a.iter().map(|c| (s(&c["check"]), s(&c["result"]), s(&c["detail"]))).collect()).unwrap_or_default(),
    }
}
