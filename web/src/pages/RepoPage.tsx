import { Link, useParams, useSearchParams } from "react-router-dom";
import { api } from "../api";
import { GitChips, GithubLink, Message } from "../components";
import { useAsync } from "../hooks";
import { ago, plural, short } from "../util";

export function RepoPage() {
  const { owner = "", name = "" } = useParams();
  const slug = `${owner}/${name}`;
  const [params] = useSearchParams();
  const page = Math.max(1, Number.parseInt(params.get("page") ?? "1", 10) || 1);
  const author = params.get("author");
  const info = useAsync(() => api.repo(slug), [slug]);
  const list = useAsync(() => api.audits(slug, page, author), [slug, page, author]);
  if (info.error) return <Message title="Cannot load repository" text={info.error} />;
  if (!info.data) return <p className="muted">Loading…</p>;
  const { repo, authors } = info.data;
  const link = (p: number) => {
    const q = new URLSearchParams({ page: String(p) });
    if (author) q.set("author", author);
    return `/${slug}?${q.toString()}`;
  };
  const total = list.data?.total ?? 0;
  const pages = Math.max(1, Math.ceil(total / (list.data?.per_page ?? 25)));
  return (
    <>
      <div className="pagehead">
        <h1>
          {owner} / <b>{name}</b>
        </h1>
        <span className="badge b-neu">repository</span>
      </div>
      <div className="tabs">
        <span className="tab on">
          Audit commits <span className="count">{list.data ? total : "…"}</span>
        </span>
      </div>
      <div className="cols">
        <div>
          {list.error && <Message title="Cannot load audit commits" text={list.error} />}
          <div className="list">
            <div className="head">
              {plural(total, "audit commit")}
              {author && (
                <>
                  {" · filtered by "}
                  <b>@{author}</b> (<Link to={`/${slug}`}>clear</Link>)
                </>
              )}
            </div>
            {list.data?.audits.length === 0 && <div className="empty">No audit commits match.</div>}
            {list.data?.audits.map((a) => (
              <div className="row" key={a.audit_id}>
                <div>
                  <div className="t">
                    <Link to={`/${slug}/audit/${a.audit_id}`}>{a.description}</Link>
                  </div>
                  <div className="m">
                    {a.author ? `@${a.author}` : "author not recorded"} · <span title={a.created_at}>{ago(a.created_at)}</span> · {plural(a.changes, "change")} · {a.gaps === 0 ? "no gaps" : plural(a.gaps, "gap")}
                  </div>
                </div>
                <div className="r">
                  <GitChips git={a.git} />
                  <br />
                  <span className="mono">{short(a.audit_id)}</span>
                </div>
              </div>
            ))}
          </div>
          {pages > 1 && (
            <div className="pager">
              {page > 1 && <Link to={link(page - 1)}>← Newer</Link>}
              <span className="muted">
                Page {page} of {pages}
              </span>
              {page < pages && <Link to={link(page + 1)}>Older →</Link>}
            </div>
          )}
        </div>
        <div className="side">
          <div className="box">
            <h3>About</h3>
            <div>{repo.remote_url ? <GithubLink url={repo.remote_url}>{repo.remote_url}</GithubLink> : <span className="muted">No GitHub remote</span>}</div>
            <div className="muted">Last push {repo.last_push_at ? <span title={repo.last_push_at}>{ago(repo.last_push_at)}</span> : "never"}</div>
            <div className="muted mono">{repo.repo_id}</div>
          </div>
          <div className="box">
            <h3>Authors</h3>
            {authors.length === 0 && <span className="muted">none recorded</span>}
            {authors.map((a) => (
              <div key={a.login}>
                <Link to={`/${slug}?author=${encodeURIComponent(a.login)}`}>@{a.login}</Link> <span className="count">{a.count}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}
