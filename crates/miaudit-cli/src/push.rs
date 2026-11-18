use crate::commands::{Ctx, Out};
use crate::error::{CliError, EXIT_INSUFFICIENT_EVIDENCE, Res};
use miaudit_schema::model::{EvidenceBody, Manifest, Stored};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, time::Duration};

fn slug_from_remote(url: &str) -> Option<String> {
    crate::git::github_web_url(url).map(|u| u.trim_start_matches("https://github.com/").to_string())
}

fn default_slug(ctx: &Ctx) -> String {
    let remote = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&ctx.store.root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| slug_from_remote(String::from_utf8_lossy(&o.stdout).trim()));
    remote.unwrap_or_else(|| {
        let dir = ctx.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
        let clean: String = dir.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
        format!("local/{}", clean.trim_start_matches('.'))
    })
}

struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl Client {
    fn send(&self, method: &str, path: &str, content_type: &str, body: &[u8]) -> Res<Value> {
        let req = self
            .agent
            .request(method, &format!("{}{path}", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", content_type);
        match req.send_bytes(body) {
            Ok(r) => r.into_json::<Value>().map_err(|e| CliError::internal(format!("server returned invalid JSON: {e}"))),
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let msg = v["error"]["message"].as_str().unwrap_or("no message").to_string();
                let rule = format!("server answered {code} {}: {msg}", v["error"]["code"].as_str().unwrap_or(""));
                Err(match code {
                    401 | 403 => CliError::validation("PUSH_UNAUTHORIZED", rule),
                    409 => CliError::conflict("PUSH_CONFLICT", rule),
                    _ => CliError::validation("PUSH_REJECTED", rule),
                })
            }
            Err(e) => Err(CliError::internal(format!("cannot reach {}: {e}", self.base))),
        }
    }
}

pub fn push(ctx: &Ctx, server: Option<&str>, name: Option<&str>) -> Res<Out> {
    let base = server
        .map(String::from)
        .or_else(|| std::env::var("MIAUDIT_SERVER").ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CliError::validation("NO_SERVER", "pass --server or export MIAUDIT_SERVER").field("/server"))?;
    let base = base.trim_end_matches('/').to_string();
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(CliError::validation("BAD_SERVER", "server must start with http:// or https://").field("/server"));
    }
    let token = std::env::var("MIAUDIT_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| CliError::validation("NO_TOKEN", "export the server token as MIAUDIT_TOKEN (never pass it as an argument)"))?;
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|p| base.starts_with(p));
    let warning = (base.starts_with("http://") && !local).then_some("server URL is plain http: the token and audit data cross the network unencrypted");

    let config = ctx.store.config()?;
    let slug = name.map(String::from).unwrap_or_else(|| default_slug(ctx));
    let remote = crate::git::repository_web_url(&ctx.store.root);
    let client = Client { agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build(), base, token };

    let commits_dir = ctx.store.dir.join("commits");
    let mut commit_ids: Vec<String> = fs::read_dir(&commits_dir)?
        .flatten()
        .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str()).map(String::from))
        .filter(|id| id.starts_with("audit_"))
        .collect();
    commit_ids.sort();

    let mut object_ids = BTreeSet::new();
    let mut blob_digests = BTreeSet::new();
    let mut unreadable = Vec::new();
    for id in &commit_ids {
        let Some(m) = ctx.store.get_commit(id).ok().flatten() else {
            unreadable.push(id.clone());
            continue;
        };
        let m: Manifest = m;
        for oid in m.context_ids.iter().chain(&m.decision_ids).chain(&m.evidence_ids) {
            object_ids.insert(oid.clone());
        }
        for eid in &m.evidence_ids {
            if let Ok(Some(ev)) = ctx.store.get_object::<EvidenceBody>(eid) {
                let ev: Stored<EvidenceBody> = ev;
                blob_digests.insert(ev.body.artifact.digest);
            }
        }
    }
    let bindings = ctx.store.list_bindings()?;
    let binding_ids: Vec<String> = bindings.iter().map(|b| b.binding_id.clone()).collect();

    client.send("PUT", &format!("/api/v1/repos/{}", config.repo_id), "application/json", &serde_json::to_vec(&json!({"slug": slug, "remote_url": remote}))?)?;

    let have = client.send(
        "POST",
        &format!("/api/v1/repos/{}/have", config.repo_id),
        "application/json",
        &serde_json::to_vec(&json!({
            "commits": commit_ids, "objects": object_ids, "blobs": blob_digests, "bindings": binding_ids,
        }))?,
    )?;
    let list = |k: &str| -> Vec<String> { have["missing"][k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default() };
    let (m_commits, m_objects, m_blobs, m_bindings) = (list("commits"), list("objects"), list("blobs"), list("bindings"));

    let repo = &config.repo_id;
    let mut missing_local = Vec::new();
    let mut sent = json!({"blobs": 0, "objects": 0, "commits": 0, "bindings": 0});
    let mut bump = |k: &str| sent[k] = json!(sent[k].as_u64().unwrap_or(0) + 1);

    for d in &m_blobs {
        match ctx.store.get_blob(d)? {
            Some(bytes) => {
                client.send("PUT", &format!("/api/v1/repos/{repo}/blobs/{d}"), "application/octet-stream", &bytes)?;
                bump("blobs");
            }
            None => missing_local.push(d.clone()),
        }
    }
    for id in &m_objects {
        match ctx.store.get_object_bytes(id)? {
            Some(bytes) => {
                client.send("PUT", &format!("/api/v1/repos/{repo}/objects/{id}"), "application/json", &bytes)?;
                bump("objects");
            }
            None => missing_local.push(id.clone()),
        }
    }
    for id in &m_commits {
        if let Some(bytes) = ctx.store.get_commit_bytes(id)? {
            client.send("PUT", &format!("/api/v1/repos/{repo}/commits/{id}"), "application/json", &bytes)?;
            bump("commits");
        }
    }
    for id in &m_bindings {
        let bytes = fs::read(ctx.store.dir.join("bindings").join(format!("{id}.json")))?;
        client.send("PUT", &format!("/api/v1/repos/{repo}/bindings/{id}"), "application/json", &bytes)?;
        bump("bindings");
    }

    let partial = !missing_local.is_empty() || !unreadable.is_empty();
    Ok(Out {
        value: json!({
            "server": client.base,
            "repo": slug,
            "repo_id": config.repo_id,
            "offered": {"commits": commit_ids.len(), "objects": object_ids.len(), "blobs": blob_digests.len(), "bindings": binding_ids.len()},
            "sent": sent,
            "already_on_server": {
                "commits": commit_ids.len() - m_commits.len(),
                "objects": object_ids.len() - m_objects.len(),
                "blobs": blob_digests.len() - m_blobs.len(),
                "bindings": binding_ids.len() - m_bindings.len(),
            },
            "missing_locally": missing_local,
            "unreadable_commits": unreadable,
            "warning": warning,
            "note": "the server re-hashes everything it stores; it does not check that any claim is true",
        }),
        exit: if partial { EXIT_INSUFFICIENT_EVIDENCE } else { 0 },
    })
}
