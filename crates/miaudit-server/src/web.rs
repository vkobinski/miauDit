
use crate::db::{AuditDoc, PAGE_SIZE, RepoDoc, RepoStats};
use miaudit_schema::model::StoredBinding;
use miaudit_ui::html::{CSS, esc, git_cell, short};

const EXTRA_CSS: &str = r#"
body{background:var(--bg)}
.topbar{background:#24292f;color:#fff;padding:12px 16px}
.topbar .in{max-width:1100px;margin:0 auto;display:flex;gap:16px;align-items:center}
.topbar a{color:#fff;font-weight:600}.topbar a:hover{text-decoration:none;opacity:.85}
.topbar form{margin-left:auto}
.topbar input{background:#32383f;border:1px solid #57606a;color:#fff;border-radius:6px;padding:5px 10px;font:inherit;font-size:13px;width:220px}
main{max-width:1100px}
.pagehead{display:flex;align-items:baseline;gap:8px;margin:0 0 16px;flex-wrap:wrap}
.pagehead h1{font-size:20px;font-weight:400}.pagehead h1 b{font-weight:600}
.tabs{display:flex;gap:4px;border-bottom:1px solid var(--line);margin-bottom:16px}
.tab{padding:8px 14px;border-bottom:2px solid transparent;color:var(--fg);font-size:14px}
.tab.on{border-color:#fd8c73;font-weight:600}
.count{background:var(--neubg);border-radius:999px;padding:0 7px;font-size:12px;margin-left:4px}
.list{background:var(--card);border:1px solid var(--line);border-radius:8px;overflow:hidden}
.list .head{padding:10px 16px;background:var(--bg);border-bottom:1px solid var(--line);font-size:13px;color:var(--mut)}
.row{display:flex;gap:16px;justify-content:space-between;align-items:flex-start;padding:14px 16px;border-bottom:1px solid var(--line)}
.row:last-child{border-bottom:0}.row .t{font-weight:600;font-size:15px}.row .m{font-size:12.5px;color:var(--mut);margin-top:3px}
.row .r{text-align:right;font-size:12.5px;color:var(--mut);white-space:nowrap}
.cols{display:grid;grid-template-columns:minmax(0,1fr) 260px;gap:24px}
.side h3{font-size:14px;margin:0 0 8px}.side .box{margin-bottom:20px;font-size:13px}.side .box div{margin:4px 0;word-break:break-all}
.pager{display:flex;justify-content:center;gap:12px;margin-top:16px;font-size:14px}
.empty{padding:32px 16px;text-align:center;color:var(--mut)}
@media(max-width:800px){.cols{grid-template-columns:1fr}.row{flex-direction:column;gap:4px}.row .r{text-align:left}.topbar input{width:140px}}
"#;

fn layout(title: &str, q: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{CSS}{EXTRA_CSS}</style></head><body>
<div class="topbar"><div class="in"><a href="/">miauDit</a><span style="opacity:.6;font-size:13px">audit commits for AI-assisted changes</span><form action="/" method="get"><input name="q" value="{}" placeholder="Find a repository…" aria-label="Find a repository"></form></div></div>
<main>{body}</main></body></html>"#,
        esc(title),
        esc(q)
    )
}

pub fn message(title: &str, text: &str) -> String {
    layout(title, "", &format!(r#"<div class="card"><h2>{}</h2><p>{}</p><p><a href="/">← All repositories</a></p></div>"#, esc(title), esc(text)))
}

pub fn ago(ts: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) else { return ts.to_string() };
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds();
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        s if s < 0 => ts.chars().take(10).collect(),
        s if s < 60 => "just now".into(),
        s if s < 3600 => plural(s / 60, "minute"),
        s if s < 86_400 => plural(s / 3600, "hour"),
        s if s < 30 * 86_400 => plural(s / 86_400, "day"),
        _ => ts.chars().take(10).collect(),
    }
}

fn when(ts: &str) -> String {
    format!(r#"<span title="{}">{}</span>"#, esc(ts), esc(&ago(ts)))
}

pub fn slug_parts(slug: &str) -> (&str, &str) {
    slug.split_once('/').unwrap_or(("", slug))
}

fn remote_link(url: &str) -> String {
    let ok = url.starts_with("https://github.com/") && url.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'));
    if ok { format!(r#"<a href="{u}" rel="noopener noreferrer" target="_blank">{u}</a>"#, u = esc(url)) } else { esc(url) }
}

pub fn home(stats: &[RepoStats], q: &str) -> String {
    let rows: String = stats
        .iter()
        .map(|s| {
            let (o, n) = slug_parts(&s.repo.slug);
            let gaps = if s.gaps == 0 { String::new() } else { format!(r#" · <span class="badge b-warn">{} gap{}</span>"#, s.gaps, if s.gaps == 1 { "" } else { "s" }) };
            format!(
                r#"<div class="row"><div><div class="t"><a href="/{slug}"><span class="muted" style="font-weight:400">{o} / </span>{n}</a></div><div class="m">{remote}</div></div><div class="r">{} audit commit{}{gaps}<br>{}</div></div>"#,
                s.audits,
                if s.audits == 1 { "" } else { "s" },
                match (&s.last_audit_at, &s.repo.last_push_at) {
                    (Some(a), _) => format!("latest audit {}", when(a)),
                    (None, Some(p)) => format!("pushed {}", when(p)),
                    _ => "nothing pushed yet".into(),
                },
                slug = esc(&s.repo.slug),
                o = esc(o),
                n = esc(n),
                remote = s.repo.remote_url.as_deref().map(remote_link).unwrap_or_else(|| r#"<span class="muted">no GitHub remote</span>"#.into()),
            )
        })
        .collect();
    let body = if rows.is_empty() {
        if q.is_empty() {
            r#"<div class="empty"><p><b>No repositories yet.</b></p><p>From a repository with a <code>.miaudit/</code> store, run:</p><pre style="display:inline-block;text-align:left">export MIAUDIT_SERVER=https://this-server\nexport MIAUDIT_TOKEN=…   # from `miaudit-server token create`\nmiaudit push</pre></div>"#.to_string()
        } else {
            format!(r#"<div class="empty">No repository matches <b>{}</b>.</div>"#, esc(q))
        }
    } else {
        rows
    };
    layout(
        "Repositories · miauDit",
        q,
        &format!(
            r#"<div class="pagehead"><h1><b>Repositories</b></h1><span class="muted">{} total</span></div>
<div class="banner">Context, decisions and evidence shown here are <b>self-reported</b> by the agents that pushed them. The server checks that every ID matches its content, not that any claim is true, and nothing is approved unless a human approval exists (none can be recorded yet).</div>
<div class="list"><div class="head">{}</div>{body}</div>"#,
            stats.len(),
            if q.is_empty() { "Sorted by most recent push".to_string() } else { format!("Matching “{}”", esc(q)) }
        ),
    )
}

pub struct RepoPage<'a> {
    pub repo: &'a RepoDoc,
    pub audits: &'a [AuditDoc],
    pub bindings: &'a [StoredBinding],
    pub total: i64,
    pub page: u64,
    pub author: Option<&'a str>,
    pub authors: &'a [(String, i64)],
}

pub fn repo(p: &RepoPage) -> String {
    let (o, n) = slug_parts(&p.repo.slug);
    let base = format!("/{}", esc(&p.repo.slug));
    let rows: String = p
        .audits
        .iter()
        .map(|a| {
            let git: String = p.bindings.iter().filter(|b| b.body.audit_id == a.audit_id).map(git_cell).collect::<Vec<_>>().join(" ");
            let git = if git.is_empty() { r#"<span class="muted">not linked to a Git commit</span>"#.to_string() } else { git };
            format!(
                r#"<div class="row"><div><div class="t"><a href="{base}/audit/{id}">{}</a></div><div class="m">{} · {} · {} change{} · {}</div></div><div class="r">{git}<br>{gaps}<span class="mono">{}</span></div></div>"#,
                esc(&a.description),
                a.author_login.as_deref().map(|l| format!("@{}", esc(l))).unwrap_or_else(|| "author not recorded".into()),
                when(&a.created_at),
                a.changes,
                if a.changes == 1 { "" } else { "s" },
                if a.gaps == 0 { "no gaps".to_string() } else { format!("{} gap{}", a.gaps, if a.gaps == 1 { "" } else { "s" }) },
                short(&a.audit_id),
                id = esc(&a.audit_id),
                gaps = "",
            )
        })
        .collect();
    let rows = if rows.is_empty() { r#"<div class="empty">No audit commits match.</div>"#.to_string() } else { rows };

    let q = |page: u64| format!("{base}?page={page}{}", p.author.map(|a| format!("&author={}", esc(a))).unwrap_or_default());
    let pages = ((p.total + PAGE_SIZE - 1) / PAGE_SIZE).max(1) as u64;
    let pager = if pages > 1 {
        format!(
            r#"<div class="pager">{}<span class="muted">Page {} of {pages}</span>{}</div>"#,
            if p.page > 1 { format!(r#"<a href="{}">← Newer</a>"#, q(p.page - 1)) } else { String::new() },
            p.page,
            if p.page < pages { format!(r#"<a href="{}">Older →</a>"#, q(p.page + 1)) } else { String::new() },
        )
    } else {
        String::new()
    };
    let authors: String = p.authors.iter().map(|(l, c)| format!(r#"<div><a href="{base}?author={l}">@{l}</a> <span class="count">{c}</span></div>"#, l = esc(l))).collect();
    let filter = p.author.map(|a| format!(r#" · filtered by <b>@{}</b> (<a href="{base}">clear</a>)"#, esc(a))).unwrap_or_default();
    let body = format!(
        r#"<div class="pagehead"><h1>{o} / <b>{n}</b></h1><span class="badge b-neu">repository</span></div>
<div class="tabs"><span class="tab on">Audit commits <span class="count">{}</span></span></div>
<div class="cols"><div><div class="list"><div class="head">{} audit commit{}{filter}</div>{rows}</div>{pager}</div>
<div class="side"><div class="box"><h3>About</h3><div>{}</div><div class="muted">Last push {}</div><div class="muted mono">{}</div></div>
<div class="box"><h3>Authors</h3>{}</div></div></div>"#,
        p.total,
        p.total,
        if p.total == 1 { "" } else { "s" },
        p.repo.remote_url.as_deref().map(remote_link).unwrap_or_else(|| r#"<span class="muted">No GitHub remote</span>"#.into()),
        p.repo.last_push_at.as_deref().map(when).unwrap_or_else(|| "never".into()),
        esc(&p.repo.repo_id),
        if authors.is_empty() { r#"<span class="muted">none recorded</span>"#.to_string() } else { authors },
        o = esc(o),
        n = esc(n),
    );
    layout(&format!("{} · miauDit", p.repo.slug), "", &body)
}

pub fn audit(repo: &RepoDoc, view: &miaudit_ui::data::View, ver: &miaudit_ui::data::Verification) -> String {
    let (o, n) = slug_parts(&repo.slug);
    let base = format!("/{}", repo.slug);
    let crumb = format!(r#"<a href="/">Repositories</a> / <a href="{b}">{} / {}</a> / <span class="mono">{}</span>"#, esc(o), esc(n), short(&view.id), b = esc(&base));
    layout(&format!("{} · {} · miauDit", view.manifest.description, repo.slug), "", &miaudit_ui::html::detail_body(view, ver, &base, &crumb))
}
