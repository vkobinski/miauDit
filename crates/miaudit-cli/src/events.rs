
use crate::{
    error::{CliError, Res},
    snapshot,
    store::Store,
};
use miaudit_schema::{
    SCHEMA_VERSION,
    canonical::to_canonical_bytes,
    digest::{DOMAIN_EVENT, content_id, domain_digest},
    model::Provenance,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const EVENTS_FILE: &str = "events.jsonl";
const CURSOR_FILE: &str = "events.cursor";
const LOCK_FILE: &str = "events.lock";
pub const MAX_HOOK_INPUT: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventBody {
    pub schema_version: String,
    pub kind: String,
    pub seq: u64,
    pub prev: Option<String>,
    pub at: String,
    pub clock_authority: String,
    pub provenance: Provenance,
    pub source: String,
    pub hook: String,
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub data: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub body: EventBody,
}

pub fn event_id(body: &EventBody) -> Res<String> {
    Ok(format!("evt_{}", domain_digest(DOMAIN_EVENT, &to_canonical_bytes(body)?)))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub enum Anchor {
    LogStart,
    Any,
}

pub fn parse_chain(bytes: &[u8], anchor: Anchor) -> Result<Vec<Event>, String> {
    let mut out: Vec<Event> = Vec::new();
    for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let ev: Event = serde_json::from_slice(line).map_err(|e| format!("line {}: not a valid event ({e})", i + 1))?;
        let want = event_id(&ev.body).map_err(|e| format!("line {}: {}", i + 1, e.rule))?;
        if want != ev.id {
            return Err(format!("line {}: id does not match content (recomputed {want})", i + 1));
        }
        match out.last() {
            Some(p) => {
                if ev.body.prev.as_deref() != Some(p.id.as_str()) || ev.body.seq != p.body.seq + 1 {
                    return Err(format!("line {}: chain link broken after seq {}", i + 1, p.body.seq));
                }
            }
            None => match anchor {
                Anchor::LogStart if ev.body.seq != 1 || ev.body.prev.is_some() => {
                    return Err(format!("line {}: first event of the log must be seq 1 without prev", i + 1));
                }
                _ => {}
            },
        }
        out.push(ev);
    }
    Ok(out)
}

pub struct Log<'a> {
    store: &'a Store,
}

impl<'a> Log<'a> {
    pub fn new(store: &'a Store) -> Self {
        Log { store }
    }

    fn path(&self) -> std::path::PathBuf {
        self.store.dir.join(EVENTS_FILE)
    }

    pub fn read_all(&self) -> Result<Vec<Event>, String> {
        match fs::read(self.path()) {
            Ok(b) => parse_chain(&b, Anchor::LogStart),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(format!("cannot read events log: {e}")),
        }
    }

    pub fn cursor(&self) -> u64 {
        fs::read_to_string(self.store.dir.join(CURSOR_FILE)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
    }

    pub fn set_cursor(&self, seq: u64) -> Res<()> {
        crate::store::write_atomic(&self.store.dir.join(CURSOR_FILE), seq.to_string().as_bytes())
    }

    pub fn since_cursor(&self) -> Result<Vec<Event>, String> {
        let c = self.cursor();
        Ok(self.read_all()?.into_iter().filter(|e| e.body.seq > c).collect())
    }

    pub fn append(&self, source: &str, hook: &str, session_id: Option<String>, tool: Option<String>, kind: &str, data: Value) -> Res<Event> {
        let lock = fs::OpenOptions::new().create(true).append(true).open(self.store.dir.join(LOCK_FILE))?;
        lock.lock()?;
        let (seq, prev) = match last_event(&self.path())? {
            Some(e) => (e.body.seq + 1, Some(e.id)),
            None => (1, None),
        };
        let body = EventBody {
            schema_version: SCHEMA_VERSION.into(),
            kind: kind.into(),
            seq,
            prev,
            at: now(),
            clock_authority: "local_device_unverified".into(),
            provenance: Provenance::Observed,
            source: source.into(),
            hook: hook.into(),
            session_id,
            tool,
            data,
        };
        let ev = Event { id: event_id(&body)?, body };
        let mut line = serde_json::to_vec(&ev)?;
        line.push(b'\n');
        let mut f = fs::OpenOptions::new().create(true).append(true).open(self.path())?;
        f.write_all(&line)?;
        f.sync_all()?;
        Ok(ev)
    }
}

fn last_event(path: &Path) -> Res<Option<Event>> {
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let len = f.metadata()?.len();
    if len == 0 {
        return Ok(None);
    }
    let take = len.min(1024 * 1024);
    f.seek(SeekFrom::Start(len - take))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let line = buf.split(|b| *b == b'\n').rev().find(|l| !l.is_empty()).ok_or_else(|| CliError::internal("events log has no readable last line"))?;
    Ok(Some(serde_json::from_slice(line).map_err(|e| CliError::internal(format!("events log tail is corrupt: {e}")))?))
}

fn str_field<'v>(v: &'v Value, k: &str) -> Option<&'v str> {
    v.get(k).and_then(Value::as_str)
}

pub fn program_of(command: &str) -> String {
    let tok = command
        .split_whitespace()
        .find(|t| {
            let name: &str = t.split('=').next().unwrap_or("");
            !(t.contains('=') && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        })
        .unwrap_or("");
    let base = tok.rsplit('/').next().unwrap_or(tok);
    base.chars().take(64).collect()
}

fn file_data(store: &Store, cwd: &Path, p: &str, operation: &str) -> Value {
    let given = Path::new(p);
    let joined = if given.is_absolute() { given.to_path_buf() } else { cwd.join(given) };
    let fixed = match (joined.parent().and_then(|d| d.canonicalize().ok()), joined.file_name()) {
        (Some(d), Some(n)) => d.join(n),
        _ => joined,
    };
    match snapshot::resolve(&store.root, cwd, &fixed.to_string_lossy()) {
        Ok((abs, rel)) => {
            let digest = fs::symlink_metadata(&abs).ok().filter(|m| m.is_file()).and_then(|_| snapshot::capture(&abs, &rel).ok()).map(|(e, _)| e.digest);
            json!({"path": rel, "operation": operation, "digest_after": digest})
        }
        Err(_) if p.ends_with(".miaudit/PROJECT.md") => {
            json!({"project_profile_edit": true, "path_as_given": p, "operation": operation})
        }
        Err(e) if e.rule.contains("excluded from capture") => {
            json!({"touches_audit_store": p.contains(".miaudit"), "touches_git_dir": p.contains(".git"), "path_as_given": p})
        }
        Err(_) => json!({"outside_repo_or_unsupported": true, "path_as_given": p}),
    }
}

const MAX_PATCH_FILES: usize = 256;

pub fn patch_files(text: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    if !text.contains("*** Begin Patch") {
        return out;
    }
    for line in text.lines() {
        let l = line.trim_end();
        if out.len() >= MAX_PATCH_FILES {
            break;
        }
        if let Some(p) = l.strip_prefix("*** Add File: ") {
            out.push(("add", p.trim().to_string()));
        } else if let Some(p) = l.strip_prefix("*** Update File: ") {
            out.push(("update", p.trim().to_string()));
        } else if let Some(p) = l.strip_prefix("*** Delete File: ") {
            out.push(("delete", p.trim().to_string()));
        } else if let Some(p) = l.strip_prefix("*** Move to: ") {
            if let Some(last) = out.last_mut()
                && last.0 == "update"
            {
                last.0 = "move_from";
            }
            out.push(("move_to", p.trim().to_string()));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    pub fn source(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude_code_hook",
            Agent::Codex => "codex_hook",
        }
    }
}

pub fn capture(store: &Store, cwd: &Path, input: &Value, agent: Agent) -> Res<Vec<Event>> {
    let log = Log::new(store);
    let hook = str_field(input, "hook_event_name").unwrap_or("unknown");
    let session = str_field(input, "session_id").map(|s| s.chars().take(128).collect::<String>());
    let tool = str_field(input, "tool_name").map(|s| s.chars().take(64).collect::<String>());
    let empty = json!({});
    let tin = input.get("tool_input").unwrap_or(&empty);
    let command_text = tin.as_str().or_else(|| str_field(tin, "command")).unwrap_or("");
    let failed = hook == "PostToolUseFailure";
    let post = matches!(hook, "PostToolUse" | "PostToolUseFailure");
    let mut items: Vec<(&str, Value)> = Vec::new();
    match (hook, tool.as_deref()) {
        ("UserPromptSubmit", _) => {
            let prompt = str_field(input, "prompt").unwrap_or("");
            let digest = store.put_blob(prompt.as_bytes())?;
            items.push(("user_prompt", json!({"prompt_blob": digest, "prompt_bytes": prompt.len() as u64})));
        }
        (_, Some("Bash")) if post => {
            let interrupted = input.get("tool_response").and_then(|r| r.get("interrupted")).and_then(Value::as_bool);
            items.push(("command", json!({"program": program_of(command_text), "command_digest": content_id(command_text.as_bytes()), "command_bytes": command_text.len() as u64, "failed": failed, "interrupted": interrupted})));
            for (op, path) in patch_files(command_text) {
                let mut d = file_data(store, cwd, &path, op);
                d["failed"] = json!(failed);
                items.push(("file_edit", d));
            }
        }
        (_, Some("apply_patch")) if post => {
            let files = patch_files(command_text);
            if files.is_empty() {
                items.push(("tool_call", json!({"failed": failed, "unparsed_patch": true})));
            }
            for (op, path) in files {
                let mut d = file_data(store, cwd, &path, op);
                d["failed"] = json!(failed);
                items.push(("file_edit", d));
            }
        }
        (_, Some(t @ ("Edit" | "Write" | "MultiEdit" | "NotebookEdit"))) if post => {
            let mut d = match str_field(tin, "file_path").or_else(|| str_field(tin, "notebook_path")) {
                Some(p) => file_data(store, cwd, p, &t.to_ascii_lowercase()),
                None => json!({"path": null, "note": "tool input has no file_path"}),
            };
            d["failed"] = json!(failed);
            items.push(("file_edit", d));
        }
        _ if post => items.push(("tool_call", json!({"failed": failed}))),
        _ => items.push(("other_hook", json!({}))),
    }
    let mut out = Vec::new();
    for (kind, data) in items {
        out.push(log.append(agent.source(), hook, session.clone(), tool.clone(), kind, data)?);
    }
    Ok(out)
}

pub fn capture_unparsed(store: &Store, reason: &str, bytes: u64, agent: Agent) -> Res<Event> {
    Log::new(store).append(agent.source(), "unparsed", None, None, "capture_error", json!({"reason": reason, "input_bytes": bytes}))
}

#[derive(Default)]
pub struct Summary {
    pub prompts: usize,
    pub commands: usize,
    pub edits: usize,
    pub last_digest: BTreeMap<String, Option<String>>,
    pub audit_store_touched: Vec<String>,
    pub capture_errors: usize,
}

pub fn summarize(events: &[Event]) -> Summary {
    let mut s = Summary::default();
    for e in events {
        match e.body.kind.as_str() {
            "user_prompt" => s.prompts += 1,
            "command" => s.commands += 1,
            "capture_error" => s.capture_errors += 1,
            "file_edit" => {
                s.edits += 1;
                if e.body.data.get("touches_audit_store").and_then(Value::as_bool) == Some(true) {
                    s.audit_store_touched.push(e.id.clone());
                }
                if let Some(p) = e.body.data.get("path").and_then(Value::as_str)
                    && e.body.data.get("failed").and_then(Value::as_bool) != Some(true)
                {
                    s.last_digest.insert(p.to_string(), e.body.data.get("digest_after").and_then(Value::as_str).map(String::from));
                }
            }
            _ => {}
        }
    }
    s
}

pub fn blob_of(events: &[Event]) -> Res<Vec<u8>> {
    let mut b = Vec::new();
    for e in events {
        b.extend(serde_json::to_vec(e)?);
        b.push(b'\n');
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_skips_env_assignments_and_directories() {
        assert_eq!(program_of("FOO=secret BAR=x ~/.cargo/bin/miaudit status"), "miaudit");
        assert_eq!(program_of("  cargo test"), "cargo");
        assert_eq!(program_of("a=b"), "");
        assert_eq!(program_of("git commit -m 'x=y'"), "git");
    }

    fn ev(seq: u64, prev: Option<&str>) -> Event {
        let body = EventBody {
            schema_version: "1".into(), kind: "command".into(), seq, prev: prev.map(String::from), at: "t".into(),
            clock_authority: "local_device_unverified".into(), provenance: Provenance::Observed, source: "claude_code_hook".into(),
            hook: "PostToolUse".into(), session_id: None, tool: None, data: json!({"n": seq}),
        };
        Event { id: event_id(&body).unwrap(), body }
    }

    fn lines(evs: &[Event]) -> Vec<u8> {
        blob_of(evs).unwrap()
    }

    #[test]
    fn chain_accepts_valid_and_rejects_tamper_gap_and_reorder() {
        let a = ev(1, None);
        let b = ev(2, Some(&a.id));
        let c = ev(3, Some(&b.id));
        assert_eq!(parse_chain(&lines(&[a.clone(), b.clone(), c.clone()]), Anchor::LogStart).unwrap().len(), 3);
        assert!(parse_chain(&lines(&[a.clone(), c.clone()]), Anchor::LogStart).is_err());
        assert!(parse_chain(&lines(&[b.clone(), a.clone()]), Anchor::LogStart).is_err());
        let mut t = b.clone();
        t.body.data = json!({"n": 99});
        assert!(parse_chain(&lines(&[a.clone(), t]), Anchor::LogStart).is_err());
        assert!(parse_chain(&lines(&[b.clone(), c.clone()]), Anchor::LogStart).is_err());
        assert_eq!(parse_chain(&lines(&[b, c]), Anchor::Any).unwrap().len(), 2);
    }
}
