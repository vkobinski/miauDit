
use miaudit_schema::{
    input::{ContextInput, DecisionInput},
    model::{EvidenceBody, Manifest, Provenance, Stored, StoredBinding, StoredCheckpoint},
};
use serde_json::Value;
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
    pub checkpoints: Vec<StoredCheckpoint>,
    pub dir: PathBuf,
    pub root: PathBuf,
    pub summaries: Vec<Summary>,
    pub unreadable: usize,
}

pub type Superseded = std::collections::BTreeMap<String, Vec<(String, String)>>;

pub struct Item<T> {
    pub id: String,
    pub object: Option<Stored<T>>,
}

pub struct EvidenceView {
    pub id: String,
    pub object: Option<Stored<EvidenceBody>>,
    pub preview: Option<String>,
    pub artifact_state: &'static str,
    pub image: bool,
}

pub struct TimelineGroup {
    pub source: &'static str,
    pub id: String,
    pub created_at: Option<String>,
    pub summary: Option<Value>,
    pub state: &'static str,
    pub events: Vec<Value>,
    pub truncated: bool,
}

pub const MAX_TIMELINE_EVENTS: usize = 3000;

pub fn events_of(bytes: &[u8]) -> (Vec<Value>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
        if out.len() >= MAX_TIMELINE_EVENTS {
            truncated = true;
            break;
        }
        let b = &v["body"];
        out.push(serde_json::json!({"id": v["id"], "seq": b["seq"], "at": b["at"], "kind": b["kind"], "tool": b["tool"], "session_id": b["session_id"], "data": b["data"]}));
    }
    (out, truncated)
}

pub struct View {
    pub timeline: Vec<TimelineGroup>,
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
        let dir = root.join(".miaudit");
        if !dir.join("config.json").is_file() {
            return Err(format!("{} is not an initialized audit repository (no .miaudit/config.json)", root.display()));
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
        let mut checkpoints: Vec<StoredCheckpoint> = fs::read_dir(dir.join("checkpoints")).into_iter().flatten().flatten().filter_map(|e| read_json(&e.path())).collect();
        checkpoints.sort_by(|a, b| (a.created.at.as_str(), a.body.first_seq).cmp(&(b.created.at.as_str(), b.body.first_seq)));
        Ok(Repo { checkpoints, dir, root: root.to_path_buf(), summaries, unreadable })
    }

    pub fn checkpoint_count(&self, audit_id: &str) -> usize {
        self.checkpoints.iter().filter(|c| c.body.audit_id == audit_id).count()
    }

    fn blob(&self, digest: &str) -> Option<Vec<u8>> {
        let hex = digest.strip_prefix("sha256:")?;
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = fs::read(self.dir.join("blobs").join(hex)).ok()?;
        (miaudit_schema::digest::content_id(&bytes) == digest).then_some(bytes)
    }

    fn group(&self, source: &'static str, id: String, created_at: Option<String>, summary: Option<Value>, digest: &str) -> TimelineGroup {
        match self.blob(digest) {
            Some(bytes) => {
                let (mut events, truncated) = events_of(&bytes);
                for e in events.iter_mut().filter(|e| e["kind"] == "user_prompt") {
                    let text = e["data"]["prompt_blob"].as_str().and_then(|d| self.blob(d)).map(|b| String::from_utf8_lossy(&b[..b.len().min(4000)]).to_string());
                    if let Some(t) = text {
                        e["prompt_text"] = Value::String(t);
                    }
                }
                TimelineGroup { source, id, created_at, summary, state: "present", events, truncated }
            }
            None => TimelineGroup { source, id, created_at, summary, state: "events unavailable", events: vec![], truncated: false },
        }
    }

    pub fn repo_id(&self) -> Option<String> {
        let config: Value = read_json(&self.dir.join("config.json"))?;
        config["repo_id"].as_str().map(String::from)
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

    pub fn evidence_image(&self, audit_id: &str, evidence_id: &str) -> Option<(&'static str, Vec<u8>)> {
        let s = self.summaries.iter().find(|s| s.id == audit_id)?;
        if !s.manifest.evidence_ids.iter().any(|e| e == evidence_id) {
            return None;
        }
        let o: Stored<EvidenceBody> = self.object("evidence", evidence_id)?;
        let digest = o.body.artifact.digest;
        let hex = digest.strip_prefix("sha256:")?;
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = fs::read(self.dir.join("blobs").join(hex)).ok()?;
        if bytes.len() > 10 * 1024 * 1024 || miaudit_schema::digest::content_id(&bytes) != digest {
            return None;
        }
        let mime = miaudit_schema::media::image_mime(&bytes)?;
        Some((mime, bytes))
    }

    pub fn superseded_by(&self, decision_ids: &[String]) -> Superseded {
        let mut out = Superseded::new();
        for s in &self.summaries {
            for did in &s.manifest.decision_ids {
                let Some(d) = self.object::<DecisionInput>("decisions", did) else { continue };
                for old in d.body.supersedes.iter().filter(|o| decision_ids.contains(o)) {
                    out.entry(old.clone()).or_default().push((did.clone(), s.id.clone()));
                }
            }
        }
        out
    }

    pub fn load(&self, id: &str) -> Option<View> {
        let s = self.summaries.iter().find(|s| s.id == id)?;
        let m = &s.manifest;
        let evidence: Vec<EvidenceView> = m.evidence_ids.iter().map(|eid| {
            let object: Option<Stored<EvidenceBody>> = self.object("evidence", eid);
            let (mut preview, mut state, mut image) = (None, "object unavailable", false);
            if let Some(o) = &object {
                let hex = o.body.artifact.digest.strip_prefix("sha256:").unwrap_or("");
                state = "artifact unavailable";
                if hex.len() == 64
                    && hex.bytes().all(|b| b.is_ascii_hexdigit())
                    && let Ok(bytes) = fs::read(self.dir.join("blobs").join(hex))
                {
                    state = "present (digest checked by verify)";
                    image = miaudit_schema::media::image_mime(&bytes).is_some();
                    if !image {
                        if bytes.len() <= 8192 {
                            preview = String::from_utf8(bytes).ok();
                        } else {
                            state = "present, too large to preview";
                        }
                    }
                }
            }
            EvidenceView { id: eid.clone(), object, preview, artifact_state: state, image }
        }).collect();
        let mut timeline: Vec<TimelineGroup> = Vec::new();
        for e in &evidence {
            if let Some(o) = e.object.as_ref().filter(|o| o.provenance == Provenance::Observed) {
                timeline.push(self.group("audit", e.id.clone(), Some(m.created.at.clone()), None, &o.body.artifact.digest));
            }
        }
        for c in self.checkpoints.iter().filter(|c| c.body.audit_id == id) {
            timeline.push(self.group("checkpoint", c.checkpoint_id.clone(), Some(c.created.at.clone()), serde_json::to_value(&c.body.summary).ok(), &c.body.events_blob));
        }
        Some(View {
            timeline,
            id: id.into(),
            manifest: m.clone(),
            bindings: s.bindings.clone(),
            contexts: m.context_ids.iter().map(|i| Item { id: i.clone(), object: self.object("contexts", i) }).collect(),
            decisions: m.decision_ids.iter().map(|i| Item { id: i.clone(), object: self.object("decisions", i) }).collect(),
            evidence,
        })
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
