import { Link, useSearchParams } from "react-router-dom";
import { api } from "../api";
import { Badge, GithubLink, Message } from "../components";
import { useAsync } from "../hooks";
import { ago, plural } from "../util";

export function Home() {
  const [params] = useSearchParams();
  const q = params.get("q") ?? "";
  const { data, error, loading } = useAsync(() => api.repos(q), [q]);
  if (error) return <Message title="Cannot load repositories" text={error} />;
  if (loading || !data) return <p className="muted">Loading…</p>;
  return (
    <>
      <div className="pagehead">
        <h1>
          <b>Repositories</b>
        </h1>
        <span className="muted">{data.length} total</span>
      </div>
      <div className="banner">
        Context, decisions and evidence shown here are <b>self-reported</b> by the agents that pushed them. The server checks that every ID matches its content, not that any claim is true, and nothing is approved unless a human approval exists (none can be recorded yet).
      </div>
      <div className="list">
        <div className="head">{q ? `Matching “${q}”` : "Sorted by most recent push"}</div>
        {data.length === 0 && (
          <div className="empty">
            {q ? (
              <>
                No repository matches <b>{q}</b>.
              </>
            ) : (
              <>
                <p>
                  <b>No repositories yet.</b>
                </p>
                <p>From a repository with a .miaudit store, run:</p>
                <pre className="inline-pre">{"export MIAUDIT_SERVER=https://this-server\nexport MIAUDIT_TOKEN=…   # from `miaudit-server token create`\nmiaudit push"}</pre>
              </>
            )}
          </div>
        )}
        {data.map((r) => {
          const [owner, name] = r.slug.split("/");
          return (
            <div className="row" key={r.repo_id}>
              <div>
                <div className="t">
                  <Link to={`/${r.slug}`}>
                    <span className="muted light">{owner} / </span>
                    {name}
                  </Link>
                </div>
                <div className="m">{r.remote_url ? <GithubLink url={r.remote_url}>{r.remote_url}</GithubLink> : <span className="muted">no GitHub remote</span>}</div>
              </div>
              <div className="r">
                {plural(r.audits, "audit commit")}
                {r.gaps > 0 && (
                  <>
                    {" · "}
                    <Badge kind="warn">{plural(r.gaps, "gap")}</Badge>
                  </>
                )}
                <br />
                {r.last_audit_at ? <span title={r.last_audit_at}>latest audit {ago(r.last_audit_at)}</span> : r.last_push_at ? <span title={r.last_push_at}>pushed {ago(r.last_push_at)}</span> : "nothing pushed yet"}
              </div>
            </div>
          );
        })}
      </div>
    </>
  );
}
