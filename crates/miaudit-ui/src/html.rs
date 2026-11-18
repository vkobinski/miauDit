
use crate::data::{Repo, Verification, View};
use miaudit_schema::model::{Correspondence, Operation, PathState, Provenance};

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

pub fn short(id: &str) -> String {
    let (head, rest) = id.split_once('_').unwrap_or(("", id));
    format!("{}{}…", if head.is_empty() { String::new() } else { format!("{head}_") }, esc(&rest.chars().take(10).collect::<String>()))
}

fn github_link(url: &str, label: &str) -> String {
    let ok = url.starts_with("https://github.com/") && url.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'));
    if ok {
        format!(r#"<a href="{}" rel="noopener noreferrer" target="_blank">{}</a>"#, esc(url), esc(label))
    } else {
        esc(label)
    }
}

pub const CSS: &str = r#"
:root{--bg:#f6f7f9;--card:#fff;--fg:#14181f;--mut:#5b6573;--line:#e2e6ec;--accent:#2457d6;--ok:#157347;--okbg:#e3f4ea;--warn:#8a5a00;--warnbg:#fff1d6;--bad:#b3261e;--badbg:#fde7e5;--neu:#46505e;--neubg:#eceff3}
@media(prefers-color-scheme:dark){:root{--bg:#0f1318;--card:#171c23;--fg:#e8ecf2;--mut:#98a3b3;--line:#2a323d;--accent:#7aa2ff;--ok:#6fd49a;--okbg:#14301f;--warn:#f0c060;--warnbg:#3a2d10;--bad:#ff8f86;--badbg:#401a18;--neu:#b4becc;--neubg:#232a34}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
main{max-width:1000px;margin:0 auto;padding:24px 16px 64px}
header.top{display:flex;gap:12px;align-items:baseline;flex-wrap:wrap;margin-bottom:4px}
h1{font-size:22px;margin:0}h2{font-size:16px;margin:0 0 12px}a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
.sub{color:var(--mut);font-size:13px;margin:0 0 20px;word-break:break-all}
.banner{background:var(--neubg);border:1px solid var(--line);border-radius:10px;padding:10px 14px;font-size:13px;color:var(--mut);margin-bottom:20px}
.card{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:16px;margin-bottom:16px;overflow-x:auto}
table{width:100%;border-collapse:collapse;font-size:14px}th,td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--line);vertical-align:top}
th{font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:var(--mut);font-weight:600}tr:last-child td{border-bottom:0}
code,.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12.5px;word-break:break-all}
.badge{display:inline-block;padding:2px 9px;border-radius:999px;font-size:12px;font-weight:600;white-space:nowrap}
.b-ok{background:var(--okbg);color:var(--ok)}.b-warn{background:var(--warnbg);color:var(--warn)}.b-bad{background:var(--badbg);color:var(--bad)}.b-neu{background:var(--neubg);color:var(--neu)}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:12px}
.stat{border:1px solid var(--line);border-radius:10px;padding:10px 12px}.stat .k{font-size:12px;color:var(--mut)}.stat .v{margin-top:4px}
.shot{max-width:100%;border:1px solid var(--line, #ccd);border-radius:6px;margin-top:8px}
.kv{display:grid;grid-template-columns:150px 1fr;gap:6px 14px;font-size:14px}.kv .k{color:var(--mut)}
.item{border-top:1px solid var(--line);padding:12px 0}.item:first-of-type{border-top:0;padding-top:0}
.muted{color:var(--mut)}pre{background:var(--bg);border:1px solid var(--line);border-radius:8px;padding:10px;overflow-x:auto;white-space:pre-wrap;word-break:break-word;font-size:12.5px;margin:6px 0 0}
ul{margin:4px 0;padding-left:20px}.crumb{font-size:13px;margin-bottom:12px}
@media(max-width:560px){.kv{grid-template-columns:1fr}}
"#;

pub fn shell(title: &str, body: &str) -> String {
    format!(r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{CSS}</style></head><body><main>{body}</main></body></html>"#, esc(title))
}

pub fn message(title: &str, text: &str) -> String {
    shell(title, &format!(r#"<div class="crumb"><a href="/">← miauDit</a></div><div class="card"><h2>{}</h2><p>{}</p></div>"#, esc(title), esc(text)))
}

pub fn prov(p: Provenance) -> String {
    let (cls, label) = match p {
        Provenance::Observed => ("b-ok", "observed"),
        Provenance::SelfReported => ("b-warn", "self-reported"),
        Provenance::Derived => ("b-neu", "derived"),
        Provenance::Unavailable => ("b-bad", "unavailable"),
        Provenance::Redacted => ("b-neu", "redacted"),
    };
    format!(r#"<span class="badge {cls}">{label}</span>"#)
}

fn corr(c: Correspondence) -> String {
    let (cls, label) = match c {
        Correspondence::Full => ("b-ok", "full match"),
        Correspondence::Partial => ("b-warn", "partial match"),
        Correspondence::NoMatch => ("b-bad", "no match"),
        Correspondence::Asserted => ("b-warn", "asserted, unverified"),
    };
    format!(r#"<span class="badge {cls}">{label}</span>"#)
}

pub fn git_cell(b: &miaudit_schema::model::StoredBinding) -> String {
    let sha: String = b.body.git.commit.chars().take(10).collect();
    let link = match &b.body.git.commit_web_url {
        Some(u) => github_link(u, &sha),
        None => esc(&sha),
    };
    let pr = b.body.pull_request.as_ref().map(|p| format!(" {}", github_link(&p.web_url, &format!("PR #{}", p.number)))).unwrap_or_default();
    format!("<code>{link}</code>{pr} {}", corr(b.body.correspondence))
}

pub fn index(repo: &Repo, port: u16) -> String {
    let mut rows = String::new();
    for s in &repo.summaries {
        let git = if s.bindings.is_empty() { r#"<span class="muted">not linked</span>"#.to_string() } else { s.bindings.iter().map(git_cell).collect::<Vec<_>>().join("<br>") };
        rows.push_str(&format!(
            r#"<tr><td class="mono">{}</td><td><a href="/audit/{id}">{}</a><div class="mono muted">{}</div></td><td>{}</td><td>{}</td><td>{}</td><td>{git}</td></tr>"#,
            esc(&s.manifest.created.at), esc(&s.manifest.description), short(&s.id),
            s.manifest.author.as_ref().and_then(|a| a.github_login.as_deref()).map(|l| format!("@{}", esc(l))).unwrap_or_else(|| r#"<span class="muted">-</span>"#.to_string()),
            s.manifest.snapshot.changes.len(), s.manifest.gaps.len(), id = esc(&s.id),
        ));
    }
    let table = if rows.is_empty() {
        r#"<p class="muted">No audit commits yet. Create one with <code>miaudit commit -m "…"</code>.</p>"#.to_string()
    } else {
        format!("<table><thead><tr><th>Created</th><th>Audit commit</th><th>GitHub user</th><th>Changes</th><th>Gaps</th><th>Git commit</th></tr></thead><tbody>{rows}</tbody></table>")
    };
    let warn = if repo.unreadable > 0 { format!(r#"<div class="banner"><span class="badge b-bad">{} unreadable</span> audit commit file(s) could not be parsed and are not listed. Run <code>miaudit verify</code>.</div>"#, repo.unreadable) } else { String::new() };
    shell("miauDit", &format!(
        r#"<header class="top"><h1>miauDit</h1><span class="muted">audit commits</span></header><p class="sub">{} · read-only view on 127.0.0.1:{port}</p>
<div class="banner">Everything here is local and unsigned. Context, decisions and evidence are <b>self-reported</b>; nothing is shown as approved unless a human approval exists, and none can be recorded yet.</div>{warn}<div class="card">{table}</div>"#,
        esc(&repo.root.display().to_string())))
}

pub fn author_cell(a: Option<&miaudit_schema::model::Author>) -> String {
    match a {
        None => r#"<span class="muted">not recorded (older audit commit)</span>"#.to_string(),
        Some(a) => match &a.github_login {
            Some(l) => format!(r#"<a href="https://github.com/{l}">@{l}</a> {} <span class="muted">({})</span>"#, prov(a.provenance), esc(&a.verification), l = esc(l)),
            None => format!(r#"{} <span class="muted">{}</span>"#, prov(a.provenance), esc(&a.verification)),
        },
    }
}

fn kv(k: &str, v: &str) -> String {
    format!(r#"<div class="k">{}</div><div>{v}</div>"#, esc(k))
}

fn list(items: &[String]) -> String {
    if items.is_empty() { return r#"<span class="muted">none</span>"#.into(); }
    format!("<ul>{}</ul>", items.iter().map(|i| format!("<li>{}</li>", esc(i))).collect::<String>())
}

pub fn detail(v: &View, ver: &Verification) -> String {
    shell(&format!("{} · miauDit", v.manifest.description), &detail_body(v, ver, "", r#"<a href="/">← all audit commits</a>"#))
}

pub fn detail_body(v: &View, ver: &Verification, base: &str, crumb: &str) -> String {
    let base = esc(base);
    let m = &v.manifest;
    let (int_cls, int_label) = match ver {
        Verification::Result { integrity, .. } => match integrity.as_str() {
            "pass" => ("b-ok", "integrity: pass".to_string()),
            "partial" => ("b-warn", "integrity: partial".to_string()),
            _ => ("b-bad", format!("integrity: {integrity}")),
        },
        Verification::Unavailable(_) => ("b-neu", "integrity: not checked".to_string()),
    };
    let stats = format!(
        r#"<div class="grid"><div class="stat"><div class="k">Accepted (stored)</div><div class="v"><span class="badge b-ok">yes</span></div></div>
<div class="stat"><div class="k">Verified</div><div class="v"><span class="badge {int_cls}">{}</span></div></div>
<div class="stat"><div class="k">Policy</div><div class="v"><span class="badge b-neu">not evaluated</span></div></div>
<div class="stat"><div class="k">Human approval</div><div class="v"><span class="badge b-bad">none recorded</span></div></div></div>"#, esc(&int_label));

    let parents = if m.parents.is_empty() { r#"<span class="muted">none (root)</span>"#.to_string() } else {
        m.parents.iter().map(|p| format!(r#"<a class="mono" href="{base}/audit/{}">{}</a>"#, esc(p), esc(p))).collect::<Vec<_>>().join("<br>")
    };
    let meta = format!(r#"<div class="card"><h2>{}</h2><div class="kv">{}{}{}{}{}{}</div></div>"#,
        esc(&m.description),
        kv("Audit ID", &format!(r#"<span class="mono">{}</span> · <a href="{base}/api/audits/{id}">raw manifest</a>"#, esc(&v.id), id = esc(&v.id))),
        kv("Created", &format!("{} <span class=\"muted\">({})</span>", esc(&m.created.at), esc(&m.created.clock_authority))),
        kv("GitHub user", &author_cell(m.author.as_ref())),
        kv("Parents", &parents),
        kv("Producer", &format!("{} {}", esc(&m.producer.tool), esc(&m.producer.version))),
        kv("Repository", &format!(r#"<span class="mono">{}</span>"#, esc(&m.repo_id))));

    let git = if v.bindings.is_empty() {
        r#"<p class="muted">Not linked to a Git commit. Link with <code>miaudit bind &lt;audit-id&gt; --git-commit &lt;rev&gt;</code>.</p>"#.to_string()
    } else {
        v.bindings.iter().map(|b| {
            let rows: String = b.body.paths.iter().map(|p| {
                let (cls, label) = match p.state {
                    PathState::Match => ("b-ok", "match"),
                    PathState::ContentDiffers => ("b-bad", "content differs"),
                    PathState::ModeDiffers => ("b-bad", "mode differs"),
                    PathState::AbsentInGit => ("b-bad", "absent in git"),
                    PathState::PresentInGit => ("b-bad", "still present in git"),
                };
                format!(r#"<tr><td class="mono">{}</td><td>{}</td><td><span class="badge {cls}">{label}</span></td></tr>"#, esc(&p.path), op(p.operation))
            }).collect();
            let repo = b.body.git.repository_web_url.as_deref().map(|u| github_link(u, u)).unwrap_or_else(|| r#"<span class="muted">no GitHub remote</span>"#.into());
            let pr_row = b.body.pull_request.as_ref().map(|p| kv("Pull request", &format!(r#"{} <span class="muted">head {}, {}</span>"#, github_link(&p.web_url, &p.web_url), esc(&p.head_commit.chars().take(10).collect::<String>()), esc(&p.state)))).unwrap_or_default();
            format!(r#"<div class="item"><div>{} {}</div><div class="kv" style="margin-top:8px">{}{}{}{}</div>
<table style="margin-top:8px"><thead><tr><th>Path</th><th>Change</th><th>In git commit</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
                git_cell(b), prov(b.body.provenance),
                kv("Git commit", &format!(r#"<span class="mono">{}</span>"#, esc(&b.body.git.commit))),
                pr_row,
                kv("Repository", &repo),
                kv("Binding", &format!(r#"<span class="mono">{}</span> · {}"#, esc(&b.binding_id), esc(&b.body.verification))))
        }).collect()
    };
    let git = format!(r#"<div class="card"><h2>Git / GitHub link</h2>{git}<p class="muted" style="font-size:13px">The web link is derived from the origin remote and does not prove the commit was pushed.</p></div>"#);

    let s = &m.snapshot;
    let gb = match &s.git_baseline {
        miaudit_schema::model::GitBaseline::Known { commit, algorithm } => format!(r#"<span class="mono">{}</span> ({})"#, esc(commit), esc(algorithm)),
        miaudit_schema::model::GitBaseline::Unavailable { reason } => format!(r#"<span class="muted">unavailable: {}</span>"#, esc(reason)),
    };
    let changes: String = s.changes.iter().map(|c| format!(
        r#"<tr><td class="mono">{}</td><td>{}</td><td class="mono">{}</td><td class="mono">{}</td></tr>"#,
        esc(&c.path), op(c.operation), esc(c.digest_after.as_deref().map(|d| &d[..d.len().min(19)]).unwrap_or("—")),
        c.mode_after.map(|m| format!("{m:o}")).unwrap_or_else(|| "—".into()))).collect();
    let snapshot = format!(r#"<div class="card"><h2>Snapshot {}</h2><div class="kv">{}{}{}{}{}</div>
<table style="margin-top:12px"><thead><tr><th>Path</th><th>Change</th><th>Digest</th><th>Mode</th></tr></thead><tbody>{}</tbody></table>
<p class="muted" style="font-size:13px">{} entries in tree · {} changes in this commit</p></div>"#,
        prov(s.provenance),
        kv("Snapshot ID", &format!(r#"<span class="mono">{}</span>"#, esc(&s.id))),
        kv("Verification", &esc(&s.verification)),
        kv("Scope", &esc(&s.scope)),
        kv("Git baseline", &gb),
        kv("Exclusions", &list(&s.exclusions)),
        if changes.is_empty() { r#"<tr><td colspan="4" class="muted">No file changes in this audit commit.</td></tr>"#.to_string() } else { changes },
        s.entries.len(), s.changes.len());

    let contexts: String = v.contexts.iter().map(|c| match &c.object {
        Some(o) => format!(r#"<div class="item"><div>{} <span class="mono muted">{}</span></div><p>{}</p><div class="kv">{}{}{}</div></div>"#,
            prov(o.provenance), short(&c.id), esc(&o.body.description),
            kv("Task ref", &o.body.task_ref.as_deref().map(esc).unwrap_or_else(|| "—".into())),
            kv("Constraints", &list(&o.body.constraints)),
            kv("Source refs", &format!("{}{}", list(&o.body.source_refs), if o.body.source_refs.is_empty() { "" } else { r#" <span class="badge b-warn">unverified</span>"# }))),
        None => format!(r#"<div class="item"><span class="mono">{}</span> <span class="badge b-bad">object unavailable</span></div>"#, esc(&c.id)),
    }).collect();

    let decisions: String = v.decisions.iter().map(|d| match &d.object {
        Some(o) => {
            let links: Vec<String> = o.body.evidence_links.iter().map(|l| format!("{:?} → {}", l.relation, l.evidence_id)).collect();
            let snap = if o.body.snapshot_id == s.id { r#"<span class="badge b-ok">matches snapshot</span>"# } else { r#"<span class="badge b-bad">different snapshot</span>"# };
            format!(r#"<div class="item"><div>{} {snap} <span class="mono muted">{}</span></div><p><b>{}</b></p><div class="kv">{}{}{}{}</div></div>"#,
                prov(o.provenance), short(&d.id), esc(&o.body.question),
                kv("Choice", &esc(&o.body.choice)), kv("Alternatives", &list(&o.body.alternatives)),
                kv("Rationale", &esc(&o.body.rationale)), kv("Risks", &list(&o.body.risks)) + &kv("Evidence links", &list(&links)))
        }
        None => format!(r#"<div class="item"><span class="mono">{}</span> <span class="badge b-bad">object unavailable</span></div>"#, esc(&d.id)),
    }).collect();

    let evidence: String = v.evidence.iter().map(|e| match &e.object {
        Some(o) => format!(r#"<div class="item"><div>{} <span class="badge b-neu">{:?}</span> <span class="mono muted">{}</span></div><p>{}</p><div class="kv">{}</div><div class="kv">{}</div>{}</div>"#,
            prov(o.provenance), o.body.classification, short(&e.id), esc(&o.body.description),
            kv("Artifact", &format!(r#"<span class="mono">{}</span> · {} bytes"#, esc(&o.body.artifact.digest), o.body.artifact.size))
                + &kv("Phase / environment", &format!("{} {}", o.body.phase.map(|p| format!(r#"<span class="badge b-neu">{p:?}</span>"#)).unwrap_or_else(|| r#"<span class="muted">not declared</span>"#.into()), o.body.environment.map(|p| format!(r#"<span class="badge b-neu">{p:?}</span>"#)).unwrap_or_default())),
            kv("Availability", &esc(e.artifact_state)),
            if e.image { format!(r#"<img class="shot" src="{base}/audit/{}/evidence/{}/image" alt="evidence image">"#, esc(&v.id), esc(&e.id)) } else { e.preview.as_deref().map(|p| format!("<pre>{}</pre>", esc(p))).unwrap_or_default() }),
        None => format!(r#"<div class="item"><span class="mono">{}</span> <span class="badge b-bad">object unavailable</span></div>"#, esc(&e.id)),
    }).collect();

    let section = |title: &str, body: String, empty: &str| format!(r#"<div class="card"><h2>{title}</h2>{}</div>"#, if body.is_empty() { format!(r#"<p class="muted">{empty}</p>"#) } else { body });

    let coverage: String = m.coverage.iter().map(|c| format!(r#"<tr><td>{}</td><td>{}</td><td class="muted">{}</td></tr>"#, esc(&c.capability), prov(c.provenance), esc(c.note.as_deref().unwrap_or("")))).collect();
    let gaps: String = m.gaps.iter().map(|g| format!(r#"<tr><td class="mono">{}</td><td>{}</td></tr>"#, esc(&g.code), esc(&g.message))).collect();
    let capture = format!(r#"<div class="card"><h2>Capture coverage</h2><table><thead><tr><th>Capability</th><th>Provenance</th><th>Note</th></tr></thead><tbody>{coverage}</tbody></table></div>
<div class="card"><h2>Gaps ({})</h2><table><thead><tr><th>Code</th><th>Meaning</th></tr></thead><tbody>{gaps}</tbody></table></div>"#, m.gaps.len());

    let verification = match ver {
        Verification::Unavailable(why) => format!(r#"<div class="card"><h2>Verification</h2><p class="muted">Not checked: {}. Run <code>miaudit verify {}</code>.</p></div>"#, esc(why), esc(&v.id)),
        Verification::Result { checks, .. } => {
            let rows: String = checks.iter().map(|(n, r, d)| {
                let cls = match r.as_str() { "pass" => "b-ok", "partial" => "b-warn", _ => "b-bad" };
                format!(r#"<tr><td>{}</td><td><span class="badge {cls}">{}</span></td><td class="mono">{}</td></tr>"#, esc(n), esc(r), esc(d))
            }).collect();
            format!(r#"<div class="card"><h2>Verification</h2><table><thead><tr><th>Check</th><th>Result</th><th>Detail</th></tr></thead><tbody>{rows}</tbody></table><p class="muted" style="font-size:13px">Local digests only, unsigned. Claims in context, decisions and evidence are not verified.</p></div>"#)
        }
    };

    format!(
        r#"<div class="crumb">{crumb}</div>{meta}{stats}<div style="height:16px"></div>{git}{snapshot}{}{}{}{capture}{verification}"#,
        section("Context", contexts, "No context recorded."),
        section("Decisions", decisions, "No decisions recorded."),
        section("Evidence", evidence, "No evidence attached."),
    )
}

fn op(o: Operation) -> &'static str {
    match o {
        Operation::Add => "add",
        Operation::Modify => "modify",
        Operation::Delete => "delete",
        Operation::TypeChange => "type change",
    }
}
