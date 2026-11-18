use crate::commands::Out;
use crate::error::{CliError, Res};
use base64::Engine;
use serde_json::{Value, json};
use std::time::Duration;

pub struct Remote {
    base: String,
    password: Option<String>,
    agent: ureq::Agent,
}

fn valid_slug(s: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.len() <= 100 && !p.starts_with('.') && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    s.split_once('/').is_some_and(|(o, n)| part(o) && part(n))
}

fn valid_audit_id(id: &str) -> bool {
    id.strip_prefix("audit_").is_some_and(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

impl Remote {
    pub fn new(server: Option<&str>) -> Res<Remote> {
        let base = server
            .map(String::from)
            .or_else(|| std::env::var("MIAUDIT_SERVER").ok())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::validation("NO_SERVER", "pass --server or export MIAUDIT_SERVER").field("/server"))?;
        let base = base.trim_end_matches('/').to_string();
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(CliError::validation("BAD_SERVER", "server must start with http:// or https://").field("/server"));
        }
        Ok(Remote {
            base,
            password: std::env::var("MIAUDIT_READ_PASSWORD").ok().filter(|p| !p.is_empty()),
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        })
    }

    fn get(&self, path: &str, query: &[(&str, String)]) -> Res<ureq::Response> {
        let mut req = self.agent.get(&format!("{}{path}", self.base));
        for (k, v) in query {
            req = req.query(k, v);
        }
        if let Some(p) = &self.password {
            req = req.set("Authorization", &format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("miaudit:{p}"))));
        }
        match req.call() {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => Err(CliError::validation("REMOTE_UNAUTHORIZED", "the server asks for the read password: export it as MIAUDIT_READ_PASSWORD")),
            Err(ureq::Error::Status(404, _)) => Err(CliError::validation("REMOTE_NOT_FOUND", format!("{path} does not exist on {}", self.base))),
            Err(ureq::Error::Status(code, _)) => Err(CliError::internal(format!("server answered {code} for {path}"))),
            Err(e) => Err(CliError::internal(format!("cannot reach {}: {e}", self.base))),
        }
    }

    fn json(&self, path: &str, query: &[(&str, String)]) -> Res<Value> {
        self.get(path, query)?.into_json().map_err(|e| CliError::internal(format!("server returned invalid JSON: {e}")))
    }
}

fn short(id: &str) -> String {
    id.chars().take(6 + 12).collect()
}

pub fn repos(r: &Remote) -> Res<Out> {
    let v = r.json("/api/v1/repos", &[])?;
    let list = v["repos"].as_array().cloned().unwrap_or_default();
    let mut lines = vec![format!("{:<36} {:>7} {:>6}  {}", "REPOSITORY", "AUDITS", "GAPS", "LATEST AUDIT")];
    for x in &list {
        lines.push(format!(
            "{:<36} {:>7} {:>6}  {}",
            x["slug"].as_str().unwrap_or("?"),
            x["audits"].as_i64().unwrap_or(0),
            x["gaps"].as_i64().unwrap_or(0),
            x["last_audit_at"].as_str().unwrap_or("-")
        ));
    }
    Ok(Out { value: json!({"server": r.base, "repos": list, "lines": lines}), exit: 0 })
}

pub struct AuditsQuery<'a> {
    pub slug: &'a str,
    pub author: Option<&'a str>,
    pub gap: Option<&'a str>,
    pub page: u64,
    pub all: bool,
}

pub fn audits(r: &Remote, q: &AuditsQuery) -> Res<Out> {
    if !valid_slug(q.slug) {
        return Err(CliError::validation("BAD_REPO", "repository must look like owner/name").field("/repo"));
    }
    let (mut audits, mut page) = (Vec::<Value>::new(), q.page.max(1));
    let mut total: u64;
    loop {
        let mut query = vec![("page", page.to_string())];
        if let Some(a) = q.author {
            query.push(("author", a.to_string()));
        }
        if let Some(g) = q.gap {
            query.push(("gap", g.to_string()));
        }
        let v = r.json(&format!("/{}/api/audits", q.slug), &query)?;
        total = v["total"].as_u64().unwrap_or(0);
        let got = v["audits"].as_array().cloned().unwrap_or_default();
        let empty = got.is_empty();
        audits.extend(got);
        if !q.all || empty || audits.len() as u64 >= total {
            break;
        }
        page += 1;
    }
    let mut lines = vec![format!("{} audit commit(s) match in {}", total, q.slug)];
    for a in &audits {
        let git = a["git"].as_array().and_then(|g| g.first()).map(|g| format!("{}({})", g["commit"].as_str().unwrap_or("?").chars().take(7).collect::<String>(), g["correspondence"].as_str().unwrap_or("?"))).unwrap_or_else(|| "not-linked".into());
        lines.push(format!(
            "{}  {}  {:<14} gaps={:<3} {:<18} {}",
            short(a["audit_id"].as_str().unwrap_or("?")),
            a["created_at"].as_str().unwrap_or("?"),
            a["author"].as_str().map(|x| format!("@{x}")).unwrap_or_else(|| "-".into()),
            a["gaps"].as_i64().unwrap_or(0),
            git,
            a["description"].as_str().unwrap_or("")
        ));
    }
    Ok(Out { value: json!({"server": r.base, "repo": q.slug, "total": total, "audits": audits, "lines": lines}), exit: 0 })
}

pub fn show(r: &Remote, slug: &str, id: &str) -> Res<Out> {
    if !valid_slug(slug) {
        return Err(CliError::validation("BAD_REPO", "repository must look like owner/name").field("/repo"));
    }
    if !valid_audit_id(id) {
        return Err(CliError::validation("BAD_ID", "audit id must be audit_ followed by 64 hex").field("/id"));
    }
    let m = r.json(&format!("/{slug}/api/audits/{id}"), &[])?;
    let mut lines = vec![
        format!("{}", m["description"].as_str().unwrap_or("")),
        format!("audit    {id}"),
        format!("created  {}", m["created"]["at"].as_str().unwrap_or("?")),
        format!("author   {}", m["author"]["github_login"].as_str().map(|l| format!("@{l}")).unwrap_or_else(|| "not recorded".into())),
        format!("parents  {}", m["parents"].as_array().map(|p| p.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()).unwrap_or_else(|| "none".into())),
        format!("changes  {}", m["snapshot"]["changes"].as_array().map(Vec::len).unwrap_or(0)),
        format!("gaps     {}", m["gaps"].as_array().map(Vec::len).unwrap_or(0)),
    ];
    for g in m["gaps"].as_array().into_iter().flatten() {
        lines.push(format!("  {}  {}", g["code"].as_str().unwrap_or("?"), g["message"].as_str().unwrap_or("")));
    }
    Ok(Out { value: json!({"server": r.base, "repo": slug, "audit_id": id, "manifest": m, "lines": lines}), exit: 0 })
}
