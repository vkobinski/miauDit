use crate::{
    error::*,
    events::{self, Log},
    git, github, project,
    snapshot::{self, EXCLUDED_DIRS},
    store::{Config, Pending, Store},
};
use miaudit_schema::{
    SCHEMA_VERSION,
    canonical::{C14N_VERSION, to_canonical_bytes},
    digest::*,
    input::*,
    model::*,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::{Path, PathBuf}, process::Command};

pub struct Out {
    pub value: Value,
    pub exit: i32,
}
fn ok(value: Value) -> Res<Out> {
    Ok(Out { value, exit: 0 })
}

pub struct Ctx {
    pub store: Store,
    pub cwd: PathBuf,
}

impl Ctx {
    pub fn open(cwd: &Path) -> Res<Ctx> {
        let cwd = cwd.canonicalize()?;
        Ok(Ctx { store: Store::discover(&cwd)?, cwd })
    }
    fn baseline(&self) -> Res<Tree> {
        match self.store.head()? {
            None => Ok(Tree::new()),
            Some(h) => {
                let m = self.store.get_commit(&h)?.ok_or_else(|| CliError::conflict("INTEGRITY_FAILURE", format!("HEAD {h} not found")))?;
                Ok(m.snapshot.entries.into_iter().map(|e| (e.path.clone(), e)).collect())
            }
        }
    }
    fn current(&self) -> Res<(Tree, Tree)> {
        let base = self.baseline()?;
        let mut cur = base.clone();
        for (p, e) in self.store.index()?.staged {
            match e {
                Some(e) => {
                    cur.insert(p, e);
                }
                None => {
                    cur.remove(&p);
                }
            }
        }
        Ok((base, cur))
    }
}

pub fn init(cwd: &Path) -> Res<Out> {
    let cwd = cwd.canonicalize()?;
    if let Ok(s) = Store::discover(&cwd) {
        let c = s.config()?;
        return ok(json!({"already_initialized": true, "repo_id": c.repo_id, "root": s.root}));
    }
    let probe = cwd.join(".audit-case-probe-x");
    let case_sensitive = fs::write(&probe, b"").ok().map(|_| !cwd.join(".AUDIT-CASE-PROBE-X").exists());
    let _ = fs::remove_file(&probe);
    let config = Config { schema_version: SCHEMA_VERSION.into(), repo_id: format!("repo_{}", uuid::Uuid::new_v4().simple()), created_at: now(), case_sensitive };
    let s = Store::init(&cwd, &config)?;
    project::init(&s)?;
    ok(json!({"already_initialized": false, "repo_id": config.repo_id, "root": s.root, "case_sensitive": case_sensitive}))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn add(ctx: &Ctx, paths: &[String]) -> Res<Out> {
    let (_, mut cur) = ctx.current()?;
    let mut index = ctx.store.index()?;
    let mut staged = Vec::new();
    for arg in paths {
        let (abs, rel) = snapshot::resolve(&ctx.store.root, &ctx.cwd, arg)?;
        let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
        let tracked_under: Vec<String> = cur.keys().filter(|k| **k == rel || k.starts_with(&prefix)).cloned().collect();
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        match fs::symlink_metadata(&abs) {
            Ok(m) if m.is_dir() => snapshot::walk(&abs, &rel, &mut files)?,
            Ok(_) => files.push((abs.clone(), rel.clone())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if tracked_under.is_empty() {
                    return Err(CliError::validation("PATH_NOT_FOUND", format!("{arg}: not found and not tracked")));
                }
            }
            Err(e) => return Err(e.into()),
        }
        let found: BTreeSet<&String> = files.iter().map(|(_, r)| r).collect();
        for gone in tracked_under.iter().filter(|t| !found.contains(t)) {
            index.staged.insert(gone.clone(), None);
            cur.remove(gone);
            staged.push(json!({"path": gone, "operation": "delete"}));
        }
        for (abs, rel) in &files {
            let (entry, bytes) = snapshot::capture(abs, rel)?;
            ctx.store.put_blob(&bytes)?;
            staged.push(json!({"path": rel, "digest": entry.digest, "mode": format!("{:o}", entry.mode)}));
            cur.insert(rel.clone(), entry.clone());
            index.staged.insert(rel.clone(), Some(entry));
        }
    }
    ctx.store.save_index(&index)?;
    let td = tree_digest(&cur)?;
    ok(json!({"staged": staged, "snapshot_id": snapshot_id(&td), "tree_digest": td}))
}

fn divergence(ctx: &Ctx) -> Res<Vec<Value>> {
    let mut out = Vec::new();
    for (p, e) in ctx.store.index()?.staged {
        let Some(staged) = e else { continue };
        let abs = ctx.store.root.join(&p);
        if fs::symlink_metadata(&abs).is_err() {
            out.push(json!({"path": p, "state": "deleted_since_staged", "staged_digest": staged.digest}));
            continue;
        }
        match snapshot::capture(&abs, &p) {
            Ok((now, _)) if now.digest != staged.digest || now.mode != staged.mode || now.kind != staged.kind => {
                out.push(json!({"path": p, "state": "modified_since_staged", "staged_digest": staged.digest, "workspace_digest": now.digest}));
            }
            Ok(_) => {}
            Err(e) => out.push(json!({"path": p, "state": "unreadable", "reason": e.rule})),
        }
    }
    Ok(out)
}

pub fn status(ctx: &Ctx) -> Res<Out> {
    let (base, cur) = ctx.current()?;
    let td = tree_digest(&cur)?;
    let pending = ctx.store.pending()?;
    let gaps = compute_gaps(ctx, &pending, &git_baseline(&ctx.store.root))?;
    ok(json!({
        "repo_id": ctx.store.config()?.repo_id,
        "head": ctx.store.head()?,
        "snapshot": {"id": snapshot_id(&td), "tree_digest": td, "entries": cur.len(), "provenance": "self_reported"},
        "staged_changes": diff_trees(&base, &cur),
        "workspace_divergence": divergence(ctx)?,
        "pending": {"contexts": pending.contexts.len(), "decisions": pending.decisions.len(), "evidence": pending.evidence.len()},
        "gaps": gaps,
    }))
}

pub fn diff(ctx: &Ctx, staged: bool) -> Res<Out> {
    if staged {
        let (base, cur) = ctx.current()?;
        ok(json!({"compare": "staged_vs_last_audit_commit", "changes": diff_trees(&base, &cur)}))
    } else {
        ok(json!({"compare": "workspace_vs_staged", "divergence": divergence(ctx)?}))
    }
}

fn read_input(file: &str) -> Res<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    let limit = 1024 * 1024 + 1;
    if file == "-" {
        std::io::stdin().take(limit).read_to_end(&mut buf)?;
    } else {
        fs::File::open(file).map_err(|e| CliError::validation("INPUT_UNREADABLE", format!("{file}: {e}")))?.take(limit).read_to_end(&mut buf)?;
    }
    if buf.len() > 1024 * 1024 {
        return Err(CliError::validation("SCHEMA_VIOLATION", "input exceeds 1 MiB").field(""));
    }
    Ok(buf)
}

fn object_id(prefix: &str, domain: &str, v: &impl serde::Serialize) -> Res<(String, Vec<u8>)> {
    let bytes = to_canonical_bytes(v)?;
    Ok((format!("{prefix}_{}", domain_digest(domain, &bytes)), bytes))
}

fn record(ctx: &Ctx, id: String, bytes: Vec<u8>, pick: fn(&mut Pending) -> &mut Vec<String>) -> Res<Out> {
    let created = ctx.store.put_object(&id, &bytes)?;
    let mut p = ctx.store.pending()?;
    let list = pick(&mut p);
    if !list.contains(&id) {
        list.push(id.clone());
        list.sort();
    }
    ctx.store.save_pending(&p)?;
    ok(json!({"id": id, "provenance": "self_reported", "duplicate": !created, "pending": true}))
}

pub fn context_add(ctx: &Ctx, file: &str) -> Res<Out> {
    let input: ContextInput = parse_input(&read_input(file)?)?;
    let (id, bytes) = object_id("ctx", DOMAIN_CONTEXT, &Stored::self_reported("context", input))?;
    record(ctx, id, bytes, |p| &mut p.contexts)
}

pub fn decision_add(ctx: &Ctx, file: &str) -> Res<Out> {
    let input: DecisionInput = parse_input(&read_input(file)?)?;
    for (i, l) in input.evidence_links.iter().enumerate() {
        if ctx.store.get_evidence(&l.evidence_id)?.is_none() {
            return Err(CliError::validation("EVIDENCE_REFERENCE_UNAVAILABLE", "evidence reference is unavailable").field(format!("/evidence_links/{i}/evidence_id")));
        }
    }
    let (id, bytes) = object_id("dec", DOMAIN_DECISION, &Stored::self_reported("decision", input))?;
    record(ctx, id, bytes, |p| &mut p.decisions)
}

pub fn evidence_add(ctx: &Ctx, file: &str) -> Res<Out> {
    let input: EvidenceInput = parse_input(&read_input(file)?)?;
    if let Some(d) = &input.decision_id
        && ctx.store.get_object::<DecisionInput>(d)?.is_none()
    {
        return Err(CliError::validation("EVIDENCE_REFERENCE_UNAVAILABLE", "decision reference is unavailable").field("/decision_id"));
    }
    let bytes = match (&input.content, &input.file) {
        (Some(c), _) => c.clone().into_bytes(),
        (_, Some(f)) => {
            let m = fs::metadata(f).map_err(|e| CliError::validation("INPUT_UNREADABLE", format!("{f}: {e}")).field("/file"))?;
            if !m.is_file() || m.len() > 10 * 1024 * 1024 {
                return Err(CliError::validation("SCHEMA_VIOLATION", "file must be a regular file up to 10 MiB").field("/file"));
            }
            fs::read(f)?
        }
        _ => unreachable!("validated"),
    };
    let digest = ctx.store.put_blob(&bytes)?;
    let body = EvidenceBody { classification: input.classification, description: input.description, artifact: ArtifactRef { digest, size: bytes.len() as u64 }, decision_id: input.decision_id, phase: input.phase, environment: input.environment };
    let (id, obj) = object_id("evd", DOMAIN_EVIDENCE, &Stored::self_reported("evidence", body))?;
    record(ctx, id, obj, |p| &mut p.evidence)
}

fn git_baseline(root: &Path) -> GitBaseline {
    match Command::new("git").args(["rev-parse", "HEAD"]).current_dir(root).output() {
        Ok(o) if o.status.success() => {
            let commit = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let algorithm = if commit.len() == 64 { "sha256" } else { "sha1" };
            GitBaseline::Known { commit, algorithm: algorithm.into() }
        }
        _ => GitBaseline::Unavailable { reason: "not_a_git_repository_or_no_commit".into() },
    }
}

const UNAVAILABLE_CAPABILITIES: [&str; 6] = [
    "conversation_capture",
    "agent_topology",
    "resource_access_receipts",
    "command_execution_receipts",
    "developer_review_observation",
    "local_encryption",
];

fn unavailable_reason(name: &str) -> &'static str {
    match name {
        "conversation_capture" => "the conversation is not captured",
        "agent_topology" => "which agents and subagents ran is not recorded",
        "resource_access_receipts" => "file reads and network access are not recorded",
        "command_execution_receipts" => "commands are not recorded",
        "developer_review_observation" => "no review or approval flow exists",
        "local_encryption" => "blobs in .miaudit/blobs are stored unencrypted",
        _ => "not implemented",
    }
}

fn partial_note(name: &str, s: &events::Summary) -> Option<&'static str> {
    match name {
        "conversation_capture" if s.prompts > 0 => Some("partial: user prompts only (hook); assistant replies and tool output are not captured"),
        "command_execution_receipts" if s.commands > 0 => Some("partial: program name and command digest only (hook); no output or exit code"),
        _ => None,
    }
}

fn coverage(obs: &Observed) -> Vec<Capability> {
    let mut c = vec![Capability { capability: "file_snapshot".into(), provenance: Provenance::SelfReported, note: Some("captured by the local CLI, not a trusted harness".into()) }];
    let trusted = obs.chain_error.is_none();
    if trusted && obs.summary.edits > 0 {
        c.push(Capability { capability: "hook_file_edits".into(), provenance: Provenance::Observed, note: Some("path and content digest after each edit tool call; shell changes are not seen".into()) });
    }
    for n in UNAVAILABLE_CAPABILITIES {
        match partial_note(n, &obs.summary).filter(|_| trusted) {
            Some(note) => c.push(Capability { capability: n.into(), provenance: Provenance::Observed, note: Some(note.into()) }),
            None => c.push(Capability { capability: n.into(), provenance: Provenance::Unavailable, note: Some(unavailable_reason(n).into()) }),
        }
    }
    c
}

struct Observed {
    events: Vec<events::Event>,
    chain_error: Option<String>,
    summary: events::Summary,
}

fn observed(ctx: &Ctx) -> Observed {
    match Log::new(&ctx.store).since_cursor() {
        Ok(events) => {
            let summary = events::summarize(&events);
            Observed { events, chain_error: None, summary }
        }
        Err(e) => Observed { events: vec![], chain_error: Some(e), summary: Default::default() },
    }
}

fn observed_gaps(ctx: &Ctx, obs: &Observed) -> Res<Vec<Gap>> {
    let mut g = Vec::new();
    if let Some(e) = &obs.chain_error {
        g.push(gap("EVENT_CHAIN_BROKEN", &format!("hook event log failed its integrity check: {e}"), vec![]));
        return Ok(g);
    }
    let (base, cur) = ctx.current()?;
    let changes: Vec<Change> = diff_trees(&base, &cur).into_iter().filter(|c| c.operation != Operation::Delete).collect();
    if obs.events.is_empty() {
        if !changes.is_empty() {
            g.push(gap("NO_OBSERVED_EVENTS", "no hook events since the last audit commit; the changes are not backed by observed capture (hooks not installed, or nothing captured)", vec![]));
        }
        return Ok(g);
    }
    let staged = ctx.store.index()?.staged;
    let last = &obs.summary.last_digest;
    let without: Vec<String> = changes.iter().filter(|c| !last.contains_key(&c.path)).map(|c| c.path.clone()).collect();
    if !without.is_empty() {
        g.push(gap("STAGED_WITHOUT_OBSERVED_EDIT", "staged paths with no observed file edit since the last audit commit (changed by a command, another tool, or outside the hooks)", without));
    }
    let differs: Vec<String> = changes.iter().filter(|c| matches!(last.get(&c.path), Some(Some(d)) if Some(d) != c.digest_after.as_ref())).map(|c| c.path.clone()).collect();
    if !differs.is_empty() {
        g.push(gap("STAGED_DIFFERS_FROM_OBSERVED", "staged content differs from the last observed edit of the path; it changed afterwards by some other means", differs));
    }
    let unstaged: Vec<String> = last.iter()
        .filter(|(p, d)| d.is_some() && !staged.contains_key(*p) && cur.get(*p).map(|e| &e.digest) != d.as_ref())
        .map(|(p, _)| p.clone()).collect();
    if !unstaged.is_empty() {
        g.push(gap("OBSERVED_EDIT_NOT_STAGED", "files edited since the last audit commit were not staged, so this audit does not cover them", unstaged));
    }
    if !obs.summary.audit_store_touched.is_empty() {
        g.push(gap("AUDIT_STORE_TOUCHED_BY_TOOL", "an editing tool wrote inside .miaudit/ or .git/; the audit store is not meant to be edited", obs.summary.audit_store_touched.clone()));
    }
    if obs.summary.capture_errors > 0 {
        g.push(gap("CAPTURE_ERRORS", "a hook delivered input that could not be recorded in full; events may be missing", vec![]));
    }
    Ok(g)
}

fn evidence_gaps(ctx: &Ctx, p: &Pending) -> Res<Vec<Gap>> {
    let mut g = Vec::new();
    let (base, cur) = ctx.current()?;
    let has_changes = !diff_trees(&base, &cur).is_empty();
    let mut kinds = BTreeSet::new();
    for id in &p.contexts {
        if let Some(c) = ctx.store.get_object::<ContextInput>(id)?
            && let Some(k) = c.body.change_kind
        {
            kinds.insert(format!("{k:?}"));
        }
    }
    if kinds.is_empty() {
        if has_changes {
            g.push(gap("CHANGE_KIND_UNDECLARED", "the context does not declare change_kind (frontend, api or other), so the evidence required for this kind of change cannot be checked", vec![]));
        }
        return Ok(g);
    }
    let mut evidence = Vec::new();
    for id in &p.evidence {
        if let Some(e) = ctx.store.get_evidence(id)? {
            let image = ctx.store.get_blob(&e.body.artifact.digest)?.is_some_and(|b| miaudit_schema::media::image_mime(&b).is_some());
            evidence.push((e.body, image));
        }
    }
    let real_env = |e: &EvidenceBody| matches!(e.environment, Some(Environment::Local | Environment::Staging | Environment::Production));
    for phase in [Phase::Before, Phase::After] {
        let word = if phase == Phase::Before { "before" } else { "after" };
        if kinds.contains("Frontend") && !evidence.iter().any(|(e, image)| e.phase == Some(phase) && real_env(e) && *image) {
            g.push(gap(&format!("FRONTEND_EVIDENCE_MISSING:{word}"), &format!("no screenshot of the UI {word} the change; ask the developer for a print, or capture one with a browser or screenshot tool, and attach it as an image with phase={word} and environment=local, staging or production"), vec![word.into()]));
        }
        if kinds.contains("Api") && !evidence.iter().any(|(e, _)| e.phase == Some(phase) && real_env(e)) {
            let what = if phase == Phase::Before { "the previous version of the code" } else { "the new version of the code" };
            g.push(gap(&format!("API_EVIDENCE_MISSING:{word}"), &format!("no evidence of {what} run against the API; attach the real requests and responses with phase={word} and environment=local, staging or production, naming the commit or snapshot that was run"), vec![word.into()]));
        }
    }
    Ok(g)
}

fn project_gaps(ctx: &Ctx) -> Vec<Gap> {
    let st = project::status(&ctx.store);
    match st.state {
        "missing" => vec![gap("PROJECT_PROFILE_MISSING", "no .miaudit/PROJECT.md: ask the developer how to run the server, collect evidence and reach staging, and record it (miaudit project init)", vec![])],
        "incomplete" => vec![gap("PROJECT_PROFILE_INCOMPLETE", "the project profile has unanswered sections; ask the developer and fill them in", st.incomplete.iter().map(|s| s.to_string()).collect())],
        "contains_secret" => vec![gap("PROJECT_PROFILE_SECRET", "the project profile looks like it contains a secret value; replace it with an environment variable name", st.secrets.iter().map(|f| format!("line {}: {}", f.line, f.kind)).collect())],
        _ => vec![],
    }
}

pub fn project_init(ctx: &Ctx) -> Res<Out> {
    let created = project::init(&ctx.store)?;
    ok(json!({"path": project::path(&ctx.store), "created": created, "sections": project::SECTIONS.iter().map(|(h, _)| h).collect::<Vec<_>>(),
        "note": "ask the developer each question, write the answers into the file, never secret values, then run `miaudit project check`"}))
}

pub fn project_check(ctx: &Ctx) -> Res<Out> {
    let st = project::status(&ctx.store);
    let exit = match st.state {
        "complete" => 0,
        "contains_secret" => EXIT_VALIDATION,
        _ => EXIT_INSUFFICIENT_EVIDENCE,
    };
    Ok(Out {
        value: json!({"path": project::path(&ctx.store), "state": st.state, "incomplete_sections": st.incomplete,
            "secret_findings": st.secrets.iter().map(|f| json!({"line": f.line, "kind": f.kind})).collect::<Vec<_>>(),
            "note": "secret detection is a heuristic on known token formats and labelled values; it does not prove the file is free of secrets"}),
        exit,
    })
}

fn gap(code: &str, message: &str, subjects: Vec<String>) -> Gap {
    Gap { code: code.into(), message: message.into(), subjects }
}

fn compute_gaps(ctx: &Ctx, p: &Pending, git: &GitBaseline) -> Res<Vec<Gap>> {
    let mut g = Vec::new();
    if p.contexts.is_empty() {
        g.push(gap("NO_CONTEXT", "no task context recorded", vec![]));
    }
    if p.decisions.is_empty() {
        g.push(gap("NO_DECISION", "no decision proposal recorded", vec![]));
    }
    let with_refs: Vec<String> = p.contexts.iter().filter_map(|id| match ctx.store.get_object::<ContextInput>(id) {
        Ok(Some(o)) if !o.body.source_refs.is_empty() => Some(id.clone()),
        _ => None,
    }).collect();
    if !with_refs.is_empty() {
        g.push(gap("UNVERIFIED_SOURCE_REFS", "source references are asserted; existence and access were not verified", with_refs));
    }
    let contradicted: Vec<String> = p.decisions.iter().filter_map(|id| match ctx.store.get_object::<DecisionInput>(id) {
        Ok(Some(o)) if o.body.evidence_links.iter().any(|l| l.relation == Relation::Contradicts) => Some(id.clone()),
        _ => None,
    }).collect();
    if !contradicted.is_empty() {
        g.push(gap("UNRESOLVED_CONTRADICTION", "decision cites evidence it marks as contradicting; no resolution recorded", contradicted));
    }
    g.push(gap("UNVERIFIED_SNAPSHOT", "snapshot was captured by the local CLI, not by a trusted harness", vec![]));
    g.push(gap("SNAPSHOT_SCOPE_STAGED_ONLY", "tree covers only staged paths; unstaged workspace files are outside this snapshot", vec![]));
    if let GitBaseline::Unavailable { .. } = git {
        g.push(gap("GIT_BASELINE_UNAVAILABLE", "no Git baseline commit could be determined", vec![]));
    }
    g.extend(project_gaps(ctx));
    g.extend(evidence_gaps(ctx, p)?);
    g.extend(observed_gaps(ctx, &observed(ctx))?);
    let obs_now = observed(ctx);
    for cap in UNAVAILABLE_CAPABILITIES {
        match partial_note(cap, &obs_now.summary).filter(|_| obs_now.chain_error.is_none()) {
            Some(note) => g.push(gap(&format!("CAPTURE_PARTIAL:{cap}"), note, vec![cap.into()])),
            None => g.push(gap(&format!("CAPTURE_UNAVAILABLE:{cap}"), "capture capability not available in this slice", vec![cap.into()])),
        }
    }
    g.push(gap("NO_HUMAN_APPROVAL", "no human disposition recorded; approvals are separate attestations", vec![]));
    Ok(g)
}

pub fn gaps(ctx: &Ctx) -> Res<Out> {
    let p = ctx.store.pending()?;
    ok(json!({"gaps": compute_gaps(ctx, &p, &git_baseline(&ctx.store.root))?}))
}

pub fn commit(ctx: &Ctx, message: &str) -> Res<Out> {
    if message.trim().is_empty() || message.chars().count() > 512 {
        return Err(CliError::validation("SCHEMA_VIOLATION", "message must be 1-512 characters").field("/message"));
    }
    let (base, cur) = ctx.current()?;
    let pending = ctx.store.pending()?;
    if diff_trees(&base, &cur).is_empty() && pending.contexts.is_empty() && pending.decisions.is_empty() && pending.evidence.is_empty() {
        return Err(CliError::validation("NOTHING_STAGED", "no staged changes and no pending context, decisions or evidence"));
    }
    let td = tree_digest(&cur)?;
    let sid = snapshot_id(&td);
    let stale: Vec<String> = pending.decisions.iter().filter_map(|id| match ctx.store.get_object::<DecisionInput>(id) {
        Ok(Some(d)) if d.body.snapshot_id != sid => Some(id.clone()),
        _ => None,
    }).collect();
    if !stale.is_empty() {
        return Err(CliError::conflict("SNAPSHOT_MISMATCH", format!("decisions reference a different snapshot than the staged one: {}", stale.join(",")))
            .expected(sid));
    }
    let git = git_baseline(&ctx.store.root);
    let config = ctx.store.config()?;
    let author = github::resolve_author();
    let obs = observed(ctx);
    let mut evidence_ids = pending.evidence.clone();
    let mut last_seq = None;
    if !obs.events.is_empty() && obs.chain_error.is_none() {
        let bytes = events::blob_of(&obs.events)?;
        let digest = ctx.store.put_blob(&bytes)?;
        let (first, last) = (obs.events.first().unwrap(), obs.events.last().unwrap());
        let body = EvidenceBody {
            classification: Classification::Confidential,
            description: format!("hook-observed events seq {}..{} ({} events, hash-chained; first prev={}, last id={}); contains user prompt blob references", first.body.seq, last.body.seq, obs.events.len(), first.body.prev.as_deref().unwrap_or("none"), last.id),
            artifact: ArtifactRef { digest, size: bytes.len() as u64 },
            decision_id: None,
            phase: None,
            environment: None,
        };
        let (id, obj) = object_id("evd", DOMAIN_EVIDENCE, &Stored::observed("evidence", body))?;
        ctx.store.put_object(&id, &obj)?;
        if !evidence_ids.contains(&id) {
            evidence_ids.push(id);
            evidence_ids.sort();
        }
        last_seq = Some(last.body.seq);
    }
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION.into(),
        canonicalization: C14N_VERSION.into(),
        kind: "audit_commit".into(),
        repo_id: config.repo_id,
        parents: ctx.store.head()?.into_iter().collect(),
        description: message.into(),
        created: Created { at: now(), clock_authority: "local_device_unverified".into() },
        producer: Producer { tool: "miaudit-cli".into(), version: env!("CARGO_PKG_VERSION").into() },
        author: Some(author.clone()),
        snapshot: Snapshot {
            id: sid.clone(),
            tree_digest: td,
            provenance: Provenance::SelfReported,
            verification: "unverified_client_snapshot".into(),
            scope: "staged_paths_only".into(),
            git_baseline: git.clone(),
            exclusions: EXCLUDED_DIRS.iter().map(|d| format!("{d}/ (any depth)")).chain(["gitignore rules are not consulted".to_string()]).collect(),
            case_sensitive: config.case_sensitive,
            entries: cur.values().cloned().collect(),
            changes: diff_trees(&base, &cur),
        },
        context_ids: pending.contexts.clone(),
        decision_ids: pending.decisions.clone(),
        evidence_ids,
        coverage: coverage(&obs),
        gaps: {
            let mut g = compute_gaps(ctx, &pending, &git)?;
            match author.provenance {
                Provenance::Unavailable => g.push(gap("AUTHOR_UNAVAILABLE", &format!("no GitHub user could be recorded: {}", author.verification), vec![])),
                Provenance::SelfReported => g.push(gap("AUTHOR_ASSERTED", "the GitHub user comes from MIAUDIT_GITHUB_USER and was not authenticated", vec![])),
                _ => g.push(gap("AUTHOR_UNSIGNED", "the GitHub user comes from the local gh session; nothing is signed", vec![])),
            }
            g
        },
    };
    let id = manifest.audit_id()?;
    ctx.store.put_commit(&id, &serde_json::to_vec_pretty(&manifest)?)?;
    ctx.store.set_head(&id)?;
    if let Some(seq) = last_seq {
        Log::new(&ctx.store).set_cursor(seq)?;
    }
    ctx.store.save_index(&Default::default())?;
    ctx.store.save_pending(&Default::default())?;
    ok(json!({
        "audit_id": id, "snapshot_id": sid,
        "accepted": true, "verified": false, "policy_passed": "not_evaluated", "human_approved": false,
        "gaps": manifest.gaps.len(), "note": "an audit commit preserves history; it does not imply policy success or approval",
    }))
}

pub fn log(ctx: &Ctx) -> Res<Out> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let bindings = ctx.store.list_bindings()?;
    let mut queue: Vec<String> = ctx.store.head()?.into_iter().collect();
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        match ctx.store.get_commit(&id)? {
            Some(m) => {
                queue.extend(m.parents.iter().cloned());
                let linked: Vec<Value> = bindings.iter().filter(|b| b.body.audit_id == id).map(|b| json!({"git_commit": b.body.git.commit, "correspondence": b.body.correspondence})).collect();
                out.push(json!({"audit_id": id, "created_at": m.created.at, "author": m.author.as_ref().and_then(|a| a.github_login.clone()), "description": m.description, "parents": m.parents, "git": linked}));
            }
            None => out.push(json!({"audit_id": id, "state": "unavailable_locally"})),
        }
    }
    ok(json!({"commits": out}))
}

pub fn show(ctx: &Ctx, id: &str) -> Res<Out> {
    let m = ctx.store.get_commit(id)?.ok_or_else(|| CliError::validation("OBJECT_NOT_FOUND", format!("{id} not found")))?;
    let load = |ids: &[String]| -> Vec<Value> {
        ids.iter().map(|i| match ctx.store.get_object_bytes(i) {
            Ok(Some(b)) => json!({"id": i, "object": serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null)}),
            _ => json!({"id": i, "state": "unavailable"}),
        }).collect()
    };
    let bindings: Vec<_> = ctx.store.list_bindings()?.into_iter().filter(|b| b.body.audit_id == id).collect();
    ok(json!({"audit_id": id, "manifest": m, "git_bindings": bindings,
        "objects": {"contexts": load(&m.context_ids), "decisions": load(&m.decision_ids), "evidence": load(&m.evidence_ids)}}))
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
enum Level {
    Pass,
    Partial,
    Fail,
}

pub fn verify(ctx: &Ctx, id: &str) -> Res<Out> {
    let bytes = ctx.store.get_commit_bytes(id)?.ok_or_else(|| CliError::validation("OBJECT_NOT_FOUND", format!("{id} not found")))?;
    let mut checks: Vec<Value> = Vec::new();
    let mut worst = Level::Pass;
    let mut note = |checks: &mut Vec<Value>, name: &str, level: Level, detail: String| {
        if level > worst {
            worst = level;
        }
        let r = match level { Level::Pass => "pass", Level::Partial => "partial", Level::Fail => "fail" };
        checks.push(json!({"check": name, "result": r, "detail": detail}));
    };
    let Ok(m) = serde_json::from_slice::<Manifest>(&bytes) else {
        note(&mut checks, "manifest_parses", Level::Fail, "manifest does not match the closed schema".into());
        return finish(id, checks, worst);
    };
    match m.audit_id() {
        Ok(got) if got == id => note(&mut checks, "audit_id_matches_canonical_manifest", Level::Pass, got),
        Ok(got) => note(&mut checks, "audit_id_matches_canonical_manifest", Level::Fail, format!("recomputed {got}")),
        Err(e) => note(&mut checks, "audit_id_matches_canonical_manifest", Level::Fail, e.to_string()),
    }
    let tree: Tree = m.snapshot.entries.iter().map(|e| (e.path.clone(), e.clone())).collect();
    match tree_digest(&tree) {
        Ok(td) if td == m.snapshot.tree_digest && snapshot_id(&td) == m.snapshot.id && tree.len() == m.snapshot.entries.len() => {
            note(&mut checks, "tree_digest", Level::Pass, td)
        }
        _ => note(&mut checks, "tree_digest", Level::Fail, "entries do not match tree_digest/snapshot id".into()),
    }
    let missing_parents: Vec<&String> = m.parents.iter().filter(|p| ctx.store.get_commit_bytes(p).ok().flatten().is_none()).collect();
    if missing_parents.is_empty() {
        note(&mut checks, "parents_available", Level::Pass, format!("{} parent(s)", m.parents.len()));
    } else {
        note(&mut checks, "parents_available", Level::Partial, format!("unavailable locally: {missing_parents:?}"));
    }
    for (name, ids, prefix, domain) in [
        ("contexts", &m.context_ids, "ctx", DOMAIN_CONTEXT),
        ("decisions", &m.decision_ids, "dec", DOMAIN_DECISION),
        ("evidence", &m.evidence_ids, "evd", DOMAIN_EVIDENCE),
    ] {
        let (mut missing, mut bad) = (vec![], vec![]);
        for oid in ids {
            match ctx.store.get_object_bytes(oid)? {
                None => missing.push(oid.clone()),
                Some(b) => {
                    let canon = serde_json::from_slice::<Value>(&b).ok().and_then(|v| to_canonical_bytes(&v).ok());
                    if canon.map(|c| format!("{prefix}_{}", domain_digest(domain, &c))).as_deref() != Some(oid.as_str()) {
                        bad.push(oid.clone());
                    }
                }
            }
        }
        if !bad.is_empty() {
            note(&mut checks, &format!("{name}_integrity"), Level::Fail, format!("content does not match id: {bad:?}"));
        } else if !missing.is_empty() {
            note(&mut checks, &format!("{name}_integrity"), Level::Partial, format!("unavailable: {missing:?}"));
        } else {
            note(&mut checks, &format!("{name}_integrity"), Level::Pass, format!("{} object(s)", ids.len()));
        }
    }
    let mut observed_bad: Vec<String> = Vec::new();
    let mut observed_missing = 0usize;
    let mut observed_events = 0usize;
    for e in &m.evidence_ids {
        let Some(ev) = ctx.store.get_evidence(e)? else { continue };
        if ev.provenance != Provenance::Observed {
            continue;
        }
        match ctx.store.get_blob(&ev.body.artifact.digest)? {
            None => observed_missing += 1,
            Some(b) => match events::parse_chain(&b, events::Anchor::Any) {
                Ok(list) => observed_events += list.len(),
                Err(why) => observed_bad.push(format!("{e}: {why}")),
            },
        }
    }
    if !observed_bad.is_empty() {
        note(&mut checks, "observed_events_chain", Level::Fail, format!("{observed_bad:?}"));
    } else if observed_missing > 0 {
        note(&mut checks, "observed_events_chain", Level::Partial, format!("{observed_missing} observed event blob(s) unavailable"));
    } else if observed_events > 0 {
        note(&mut checks, "observed_events_chain", Level::Pass, format!("{observed_events} event(s): ids match content and links are contiguous (proves internal consistency, not that the log is complete)"));
    }
    let bad_bindings: Vec<String> = ctx.store.list_bindings()?.into_iter()
        .filter(|b| b.body.audit_id == id && (b.body.binding_id().ok().as_deref() != Some(b.binding_id.as_str())))
        .map(|b| b.binding_id).collect();
    if bad_bindings.is_empty() {
        note(&mut checks, "git_bindings_integrity", Level::Pass, "binding ids match their content".into());
    } else {
        note(&mut checks, "git_bindings_integrity", Level::Fail, format!("altered binding(s): {bad_bindings:?}"));
    }
    let mut blobs: BTreeSet<String> = m.snapshot.entries.iter().map(|e| e.digest.clone()).collect();
    for e in &m.evidence_ids {
        if let Some(ev) = ctx.store.get_evidence(e)? {
            blobs.insert(ev.body.artifact.digest);
        }
    }
    let (mut missing, mut bad) = (vec![], vec![]);
    for d in &blobs {
        match ctx.store.get_blob(d)? {
            None => missing.push(d.clone()),
            Some(b) if content_id(&b) != *d => bad.push(d.clone()),
            Some(_) => {}
        }
    }
    if !bad.is_empty() {
        note(&mut checks, "artifacts_integrity", Level::Fail, format!("altered artifact(s): {bad:?}"));
    } else if !missing.is_empty() {
        note(&mut checks, "artifacts_integrity", Level::Partial, format!("{} artifact(s) unavailable: content missing is not an empty file", missing.len()));
    } else {
        note(&mut checks, "artifacts_integrity", Level::Pass, format!("{} artifact(s)", blobs.len()));
    }
    finish(id, checks, worst)
}

pub fn bind(ctx: &Ctx, audit_id: &str, rev: &str, assert: bool) -> Res<Out> {
    bind_inner(ctx, audit_id, rev, assert, true)
}

fn bind_inner(ctx: &Ctx, audit_id: &str, rev: &str, assert: bool, record: bool) -> Res<Out> {
    let m = ctx.store.get_commit(audit_id)?.ok_or_else(|| CliError::validation("OBJECT_NOT_FOUND", format!("{audit_id} not found")))?;
    let root = &ctx.store.root;
    let (commit, algorithm) = git::resolve_commit(root, rev)?;
    let mut paths = Vec::new();
    for ch in &m.snapshot.changes {
        paths.push(PathCheck { path: ch.path.clone(), operation: ch.operation, state: git::check(root, &commit, ch)? });
    }
    let matched = paths.iter().filter(|p| p.state == PathState::Match).count();
    let correspondence = match (paths.len(), matched) {
        (0, _) if assert => Correspondence::Asserted,
        (0, _) => return Err(CliError::new("BINDING_UNVERIFIABLE", EXIT_INSUFFICIENT_EVIDENCE, "the audit commit has no file changes to compare; use --assert to record an unverified link")),
        (n, k) if k == n => Correspondence::Full,
        (_, 0) if assert => Correspondence::Asserted,
        (_, 0) => {
            let detail: Vec<String> = paths.iter().map(|p| format!("{}:{:?}", p.path, p.state)).collect();
            return Err(CliError::conflict("BINDING_MISMATCH", format!("none of the audit commit's changes match git commit {commit}: {}", detail.join(", ")))
                .expected("use --assert to record an unverified link"));
        }
        _ => Correspondence::Partial,
    };
    let repo_url = git::repository_web_url(root);
    let body = GitBindingBody {
        schema_version: SCHEMA_VERSION.into(),
        kind: "git_binding".into(),
        audit_id: audit_id.into(),
        git: GitRef { commit_web_url: repo_url.as_ref().map(|u| format!("{u}/commit/{commit}")), repository_web_url: repo_url, commit, algorithm },
        correspondence,
        verification: "local_git_inspection_unsigned".into(),
        provenance: if correspondence == Correspondence::Asserted { Provenance::SelfReported } else { Provenance::Derived },
        paths,
    };
    let binding_id = body.binding_id()?;
    let stored = StoredBinding { binding_id: binding_id.clone(), body, created: Created { at: now(), clock_authority: "local_device_unverified".into() } };
    let created = if record { ctx.store.put_binding(&binding_id, &serde_json::to_vec_pretty(&stored)?)? } else { false };
    ok(json!({
        "binding_id": binding_id, "audit_id": audit_id, "duplicate": !created,
        "git_commit": stored.body.git.commit, "commit_web_url": stored.body.git.commit_web_url,
        "correspondence": correspondence, "paths_matched": matched, "paths_total": stored.body.paths.len(),
        "paths": stored.body.paths,
        "note": "web link is derived from the origin remote; it does not prove the commit was pushed",
    }))
}

pub fn bind_pending(ctx: &Ctx, rev: &str) -> Res<Out> {
    let bound: BTreeSet<String> = ctx.store.list_bindings()?.into_iter().map(|b| b.body.audit_id).collect();
    let mut linked = Vec::new();
    let mut skipped = Vec::new();
    let mut queue: Vec<String> = ctx.store.head()?.into_iter().collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(m) = ctx.store.get_commit(&id)? else { continue };
        queue.extend(m.parents.iter().cloned());
        if bound.contains(&id) {
            continue;
        }
        match bind_inner(ctx, &id, rev, false, false) {
            Ok(Out { value, .. }) if value["correspondence"] == "full" => {
                let done = bind_inner(ctx, &id, rev, false, true)?;
                linked.push(json!({"audit_id": id, "correspondence": "full", "binding_id": done.value["binding_id"]}));
            }
            Ok(Out { value, .. }) => skipped.push(json!({"audit_id": id, "reason": format!("correspondence is {}; only full matches are linked automatically", value["correspondence"].as_str().unwrap_or("unknown"))})),
            Err(e) => skipped.push(json!({"audit_id": id, "reason": e.code})),
        }
    }
    ok(json!({"git_rev": rev, "linked": linked, "skipped": skipped,
        "note": "audit commits that were already linked are left alone; a partial binding is recorded only by running `miaudit bind` yourself"}))
}

const POST_COMMIT_HOOK: &str = "#!/bin/sh\n\"$HOME/.cargo/bin/miaudit\" bind-pending --git-commit HEAD >/dev/null 2>&1 || true\n";

pub fn git_hook_install(ctx: &Ctx) -> Res<Out> {
    use std::os::unix::fs::PermissionsExt;
    let o = Command::new("git").args(["rev-parse", "--git-path", "hooks/post-commit"]).current_dir(&ctx.store.root).output()
        .map_err(|e| CliError::validation("GIT_UNAVAILABLE", format!("git could not be run: {e}")))?;
    if !o.status.success() {
        return Err(CliError::validation("NOT_A_GIT_REPOSITORY", "this directory is not inside a Git repository"));
    }
    let rel = String::from_utf8_lossy(&o.stdout).trim().to_string();
    let path = ctx.store.root.join(&rel);
    match fs::read_to_string(&path) {
        Ok(existing) if existing == POST_COMMIT_HOOK => return ok(json!({"hook": path, "installed": true, "changed": false})),
        Ok(_) => return Err(CliError::conflict("HOOK_EXISTS", format!("{} already exists with other content; not overwriting", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, POST_COMMIT_HOOK)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    ok(json!({"hook": path, "installed": true, "changed": true,
        "note": "after each git commit, audit commits that match it fully are linked; partial matches are never linked automatically"}))
}

pub fn bindings(ctx: &Ctx, audit_id: Option<&str>) -> Res<Out> {
    let list: Vec<_> = ctx.store.list_bindings()?.into_iter().filter(|b| audit_id.is_none_or(|a| b.body.audit_id == a)).collect();
    ok(json!({"bindings": list}))
}

pub fn related(ctx: &Ctx, rev: &str) -> Res<Out> {
    let commit = match git::resolve_commit(&ctx.store.root, rev) {
        Ok((c, _)) => c,
        Err(e) if rev.len() >= 7 && rev.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let _ = e;
            rev.to_ascii_lowercase()
        }
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for b in ctx.store.list_bindings()?.into_iter().filter(|b| b.body.git.commit.starts_with(&commit)) {
        let m = ctx.store.get_commit(&b.body.audit_id)?;
        out.push(json!({
            "audit_id": b.body.audit_id, "binding_id": b.binding_id, "correspondence": b.body.correspondence,
            "description": m.as_ref().map(|m| m.description.clone()),
            "paths": b.body.paths, "commit_web_url": b.body.git.commit_web_url,
        }));
    }
    ok(json!({"git_commit": commit, "audits": out}))
}

pub fn events_status(ctx: &Ctx) -> Res<Out> {
    let log = Log::new(&ctx.store);
    let cursor = log.cursor();
    match log.read_all() {
        Ok(all) => {
            let pending: Vec<&events::Event> = all.iter().filter(|e| e.body.seq > cursor).collect();
            let recent: Vec<Value> = all.iter().rev().take(20).rev().map(|e| json!({
                "seq": e.body.seq, "id": e.id, "at": e.body.at, "hook": e.body.hook, "kind": e.body.kind, "tool": e.body.tool, "data": e.body.data,
            })).collect();
            ok(json!({"chain": "ok", "events_total": all.len(), "cursor": cursor, "events_since_last_audit_commit": pending.len(), "recent": recent,
                "provenance": "observed", "note": "the chain proves internal consistency; whoever can write .miaudit/ can still rewrite the whole log and recompute it"}))
        }
        Err(why) => Ok(Out { value: json!({"chain": "broken", "detail": why, "cursor": cursor}), exit: EXIT_VALIDATION }),
    }
}

pub fn stop_check(ctx: &Ctx) -> Option<String> {
    let obs = observed(ctx);
    if obs.chain_error.is_some() {
        return Some("miaudit: the hook event log failed its integrity check (see `miaudit events`). Tell the user before finishing.".into());
    }
    let edited: Vec<&String> = obs.summary.last_digest.keys().collect();
    if edited.is_empty() {
        return None;
    }
    let shown: Vec<&str> = edited.iter().take(8).map(|s| s.as_str()).collect();
    Some(format!(
        "miaudit: {} file(s) edited since the last audit commit and not yet audited ({}{}). Follow the audit workflow in CLAUDE.md (add, context, decision, evidence, commit, verify) or tell the user why you are not auditing this work.",
        edited.len(), shown.join(", "), if edited.len() > shown.len() { ", ..." } else { "" }
    ))
}

fn finish(id: &str, checks: Vec<Value>, worst: Level) -> Res<Out> {
    let (integrity, exit) = match worst {
        Level::Pass => ("pass", 0),
        Level::Partial => ("partial", EXIT_INSUFFICIENT_EVIDENCE),
        Level::Fail => ("fail", EXIT_VALIDATION),
    };
    Ok(Out {
        value: json!({
            "audit_id": id,
            "accepted": true,
            "verified": {"integrity": integrity, "signatures": "none", "scope": "local digests only; contents of claims are not verified"},
            "policy_passed": "not_evaluated",
            "human_approved": false,
            "checks": checks,
        }),
        exit,
    })
}
