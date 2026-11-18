use crate::data::{Superseded, Verification, View};
use miaudit_schema::model::StoredBinding;
use serde_json::{Value, json};

pub struct RowFields<'a> {
    pub audit_id: &'a str,
    pub created_at: &'a str,
    pub description: &'a str,
    pub author: Option<&'a str>,
    pub changes: i64,
    pub gaps: i64,
    pub gap_codes: &'a [String],
    pub parents: &'a [String],
}

pub fn git_links(bindings: &[StoredBinding], audit_id: &str) -> Vec<Value> {
    bindings
        .iter()
        .filter(|b| b.body.audit_id == audit_id)
        .map(|b| {
            let body = serde_json::to_value(&b.body).unwrap_or(Value::Null);
            json!({
                "commit": b.body.git.commit,
                "correspondence": b.body.correspondence,
                "url": b.body.git.commit_web_url,
                "pull_request": body.get("pull_request").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

pub fn audit_row(f: &RowFields, git: Vec<Value>) -> Value {
    json!({
        "audit_id": f.audit_id, "created_at": f.created_at, "description": f.description, "author": f.author,
        "changes": f.changes, "gaps": f.gaps, "gap_codes": f.gap_codes, "parents": f.parents, "git": git,
    })
}

pub fn page_of<T>(rows: &[T], page: u64, per_page: u64) -> &[T] {
    let start = (page.saturating_sub(1) * per_page) as usize;
    if start >= rows.len() {
        return &[];
    }
    &rows[start..rows.len().min(start + per_page as usize)]
}

pub fn detail_json(v: &View, ver: &Verification, superseded: &Superseded) -> Value {
    let mut manifest = serde_json::to_value(&v.manifest).unwrap_or(Value::Null);
    if let Some(snapshot) = manifest.get_mut("snapshot").and_then(Value::as_object_mut) {
        let count = snapshot.remove("entries").and_then(|e| e.as_array().map(Vec::len)).unwrap_or(0);
        snapshot.insert("entries_count".into(), json!(count));
    }
    let verification = match ver {
        Verification::Unavailable(reason) => json!({"state": "unavailable", "reason": reason}),
        Verification::Result { integrity, checks } => json!({
            "state": "result",
            "integrity": integrity,
            "checks": checks.iter().map(|(check, result, detail)| json!({"check": check, "result": result, "detail": detail})).collect::<Vec<_>>(),
        }),
    };
    json!({
        "audit_id": v.id,
        "manifest": manifest,
        "bindings": v.bindings,
        "contexts": v.contexts.iter().map(|i| json!({"id": i.id, "object": i.object})).collect::<Vec<_>>(),
        "decisions": v.decisions.iter().map(|i| json!({
            "id": i.id,
            "object": i.object,
            "superseded_by": superseded.get(&i.id).map(|l| l.iter().map(|(decision_id, audit_id)| json!({"decision_id": decision_id, "audit_id": audit_id})).collect::<Vec<_>>()).unwrap_or_default(),
        })).collect::<Vec<_>>(),
        "evidence": v.evidence.iter().map(|e| json!({"id": e.id, "object": e.object, "preview": e.preview, "artifact_state": e.artifact_state, "image": e.image})).collect::<Vec<_>>(),
        "verification": verification,
    })
}
