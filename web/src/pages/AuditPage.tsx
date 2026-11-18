import type { ReactNode } from "react";
import { Link, useParams } from "react-router-dom";
import { api } from "../api";
import { Badge, BulletList, CorrBadge, GithubLink, Kv, Message, ProvBadge } from "../components";
import { useAsync } from "../hooks";
import type { AuditDetail, Binding, Change, PathCheck } from "../types";
import { short } from "../util";

const OPERATION: Record<Change["operation"], string> = { add: "add", modify: "modify", delete: "delete", type_change: "type change" };

const PATH_STATE: Record<PathCheck["state"], ["ok" | "bad", string]> = {
  match: ["ok", "match"],
  content_differs: ["bad", "content differs"],
  mode_differs: ["bad", "mode differs"],
  absent_in_git: ["bad", "absent in git"],
  present_in_git: ["bad", "still present in git"],
};

function Card({ title, children }: { title: ReactNode; children: ReactNode }) {
  return (
    <div className="card">
      <h2>{title}</h2>
      {children}
    </div>
  );
}

function Section({ title, empty, children }: { title: string; empty: string; children: ReactNode[] }) {
  return <Card title={title}>{children.length === 0 ? <p className="muted">{empty}</p> : children}</Card>;
}

function Unavailable({ id }: { id: string }) {
  return (
    <div className="item">
      <span className="mono">{id}</span> <Badge kind="bad">object unavailable</Badge>
    </div>
  );
}

function BindingView({ b }: { b: Binding }) {
  const g = b.body.git;
  const pr = b.body.pull_request;
  return (
    <div className="item">
      <div>
        <code>
          <GithubLink url={g.commit_web_url}>{g.commit.slice(0, 10)}</GithubLink>
        </code>
        {pr && (
          <>
            {" "}
            <GithubLink url={pr.web_url}>{`PR #${pr.number}`}</GithubLink>
          </>
        )}{" "}
        <CorrBadge c={b.body.correspondence} /> <ProvBadge p={b.body.provenance} />
      </div>
      <div className="kv mt8">
        <Kv k="Git commit">
          <span className="mono">{g.commit}</span>
        </Kv>
        {pr && (
          <Kv k="Pull request">
            <GithubLink url={pr.web_url}>{pr.web_url}</GithubLink> <span className="muted">head {pr.head_commit.slice(0, 10)}, {pr.state}</span>
          </Kv>
        )}
        <Kv k="Repository">{g.repository_web_url ? <GithubLink url={g.repository_web_url}>{g.repository_web_url}</GithubLink> : <span className="muted">no GitHub remote</span>}</Kv>
        <Kv k="Binding">
          <span className="mono">{b.binding_id}</span> · {b.body.verification}
        </Kv>
      </div>
      <table className="mt8">
        <thead>
          <tr>
            <th>Path</th>
            <th>Change</th>
            <th>In git commit</th>
          </tr>
        </thead>
        <tbody>
          {b.body.paths.map((p) => {
            const [kind, label] = PATH_STATE[p.state];
            return (
              <tr key={p.path}>
                <td className="mono">{p.path}</td>
                <td>{OPERATION[p.operation]}</td>
                <td>
                  <Badge kind={kind}>{label}</Badge>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function AuditView({ slug, d }: { slug: string; d: AuditDetail }) {
  const m = d.manifest;
  const [owner, name] = slug.split("/");
  const v = d.verification;
  const integrity = v.state === "result" ? v.integrity : null;
  const integrityKind = integrity === "pass" ? "ok" : integrity === "partial" ? "warn" : integrity === null ? "neu" : "bad";
  const s = m.snapshot;
  return (
    <>
      <div className="crumb">
        <Link to="/">Repositories</Link> / <Link to={`/${slug}`}>{owner} / {name}</Link> / <span className="mono">{short(d.audit_id)}</span>
      </div>
      <Card title={m.description}>
        <div className="kv">
          <Kv k="Audit ID">
            <span className="mono">{d.audit_id}</span> · <a href={api.rawUrl(slug, d.audit_id)}>raw manifest</a>
          </Kv>
          <Kv k="Created">
            {m.created.at} <span className="muted">({m.created.clock_authority})</span>
          </Kv>
          <Kv k="GitHub user">
            {m.author ? (
              m.author.github_login ? (
                <>
                  <GithubLink url={`https://github.com/${m.author.github_login}`}>@{m.author.github_login}</GithubLink> <ProvBadge p={m.author.provenance} /> <span className="muted">({m.author.verification})</span>
                </>
              ) : (
                <>
                  <ProvBadge p={m.author.provenance} /> <span className="muted">{m.author.verification}</span>
                </>
              )
            ) : (
              <span className="muted">not recorded (older audit commit)</span>
            )}
          </Kv>
          <Kv k="Parents">
            {m.parents.length === 0 ? (
              <span className="muted">none (root)</span>
            ) : (
              m.parents.map((p) => (
                <div key={p}>
                  <Link className="mono" to={`/${slug}/audit/${p}`}>
                    {p}
                  </Link>
                </div>
              ))
            )}
          </Kv>
          <Kv k="Producer">
            {m.producer.tool} {m.producer.version}
          </Kv>
          <Kv k="Repository">
            <span className="mono">{m.repo_id}</span>
          </Kv>
        </div>
      </Card>

      <div className="grid">
        <div className="stat">
          <div className="k">Accepted (stored)</div>
          <div className="v">
            <Badge kind="ok">yes</Badge>
          </div>
        </div>
        <div className="stat">
          <div className="k">Verified</div>
          <div className="v">
            <Badge kind={integrityKind}>{integrity === null ? "integrity: not checked" : `integrity: ${integrity}`}</Badge>
          </div>
        </div>
        <div className="stat">
          <div className="k">Policy</div>
          <div className="v">
            <Badge kind="neu">not evaluated</Badge>
          </div>
        </div>
        <div className="stat">
          <div className="k">Human approval</div>
          <div className="v">
            <Badge kind="bad">none recorded</Badge>
          </div>
        </div>
      </div>
      <div className="spacer" />

      <Card title="Git / GitHub link">
        {d.bindings.length === 0 ? (
          <p className="muted">
            Not linked to a Git commit or pull request. Link with <code>miaudit bind &lt;audit-id&gt; --git-commit &lt;rev&gt;</code>.
          </p>
        ) : (
          d.bindings.map((b) => <BindingView key={b.binding_id} b={b} />)
        )}
        <p className="muted small">The web link is derived from the origin remote and does not prove the commit was pushed.</p>
      </Card>

      <Card
        title={
          <>
            Snapshot <ProvBadge p={s.provenance} />
          </>
        }
      >
        <div className="kv">
          <Kv k="Snapshot ID">
            <span className="mono">{s.id}</span>
          </Kv>
          <Kv k="Verification">{s.verification}</Kv>
          <Kv k="Scope">{s.scope}</Kv>
          <Kv k="Git baseline">
            {s.git_baseline.state === "known" ? (
              <>
                <span className="mono">{s.git_baseline.commit}</span> ({s.git_baseline.algorithm})
              </>
            ) : (
              <span className="muted">unavailable: {s.git_baseline.reason}</span>
            )}
          </Kv>
          <Kv k="Exclusions">
            <BulletList items={s.exclusions} />
          </Kv>
        </div>
        <table className="mt12">
          <thead>
            <tr>
              <th>Path</th>
              <th>Change</th>
              <th>Digest</th>
              <th>Mode</th>
            </tr>
          </thead>
          <tbody>
            {s.changes.length === 0 && (
              <tr>
                <td colSpan={4} className="muted">
                  No file changes in this audit commit.
                </td>
              </tr>
            )}
            {s.changes.map((c) => (
              <tr key={c.path}>
                <td className="mono">{c.path}</td>
                <td>{OPERATION[c.operation]}</td>
                <td className="mono">{c.digest_after ? c.digest_after.slice(0, 19) : "—"}</td>
                <td className="mono">{c.mode_after === null ? "—" : c.mode_after.toString(8)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="muted small">
          {s.entries_count} entries in tree · {s.changes.length} changes in this commit
        </p>
      </Card>

      <Section title="Context" empty="No context recorded.">
        {d.contexts.map((c) =>
          c.object ? (
            <div className="item" key={c.id}>
              <div>
                <ProvBadge p={c.object.provenance} /> <span className="mono muted">{short(c.id)}</span>
              </div>
              <p>{c.object.body.description}</p>
              <div className="kv">
                <Kv k="Task ref">{c.object.body.task_ref ?? "—"}</Kv>
                <Kv k="Constraints">
                  <BulletList items={c.object.body.constraints} />
                </Kv>
                <Kv k="Source refs">
                  <BulletList items={c.object.body.source_refs} />
                  {c.object.body.source_refs.length > 0 && <Badge kind="warn">unverified</Badge>}
                </Kv>
              </div>
            </div>
          ) : (
            <Unavailable key={c.id} id={c.id} />
          ),
        )}
      </Section>

      <Section title="Decisions" empty="No decisions recorded.">
        {d.decisions.map((x) =>
          x.object ? (
            <div className="item" key={x.id}>
              <div>
                <ProvBadge p={x.object.provenance} /> {x.object.body.snapshot_id === s.id ? <Badge kind="ok">matches snapshot</Badge> : <Badge kind="bad">different snapshot</Badge>} {x.superseded_by.length > 0 && <Badge kind="warn">superseded</Badge>} <span className="mono muted">{short(x.id)}</span>
              </div>
              {x.superseded_by.map((n) => (
                <p className="small" key={n.decision_id}>
                  Superseded by <span className="mono">{short(n.decision_id)}</span> in <Link to={`/${slug}/audit/${n.audit_id}`}>{short(n.audit_id)}</Link>. This record is unchanged; read the newer one for the current position.
                </p>
              ))}
              <p>
                <b>{x.object.body.question}</b>
              </p>
              <div className="kv">
                <Kv k="Choice">{x.object.body.choice}</Kv>
                <Kv k="Alternatives">
                  <BulletList items={x.object.body.alternatives} />
                </Kv>
                <Kv k="Rationale">{x.object.body.rationale}</Kv>
                <Kv k="Risks">
                  <BulletList items={x.object.body.risks} />
                </Kv>
                <Kv k="Evidence links">
                  <BulletList items={x.object.body.evidence_links.map((l) => `${l.relation} → ${l.evidence_id}`)} />
                </Kv>
                {(x.object.body.supersedes ?? []).length > 0 && (
                  <Kv k="Supersedes">
                    <BulletList items={x.object.body.supersedes ?? []} />
                  </Kv>
                )}
              </div>
            </div>
          ) : (
            <Unavailable key={x.id} id={x.id} />
          ),
        )}
      </Section>

      <Section title="Evidence" empty="No evidence attached.">
        {d.evidence.map((e) =>
          e.object ? (
            <div className="item" key={e.id}>
              <div>
                <ProvBadge p={e.object.provenance} /> <Badge kind="neu">{e.object.body.classification}</Badge> <span className="mono muted">{short(e.id)}</span>
              </div>
              <p>{e.object.body.description}</p>
              <div className="kv">
                <Kv k="Artifact">
                  <span className="mono">{e.object.body.artifact.digest}</span> · {e.object.body.artifact.size} bytes
                </Kv>
                <Kv k="Phase / environment">
                  {e.object.body.phase ? <Badge kind="neu">{e.object.body.phase}</Badge> : <span className="muted">not declared</span>} {e.object.body.environment && <Badge kind="neu">{e.object.body.environment}</Badge>}
                </Kv>
              </div>
              <div className="kv">
                <Kv k="Availability">{e.artifact_state}</Kv>
              </div>
              {e.image ? <img className="shot" src={api.imageUrl(slug, d.audit_id, e.id)} alt="evidence" /> : e.preview !== null && <pre>{e.preview}</pre>}
            </div>
          ) : (
            <Unavailable key={e.id} id={e.id} />
          ),
        )}
      </Section>

      <Card title="Capture coverage">
        <table>
          <thead>
            <tr>
              <th>Capability</th>
              <th>Provenance</th>
              <th>Note</th>
            </tr>
          </thead>
          <tbody>
            {m.coverage.map((c) => (
              <tr key={c.capability}>
                <td>{c.capability}</td>
                <td>
                  <ProvBadge p={c.provenance} />
                </td>
                <td className="muted">{c.note ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
      <Card title={`Gaps (${m.gaps.length})`}>
        <table>
          <thead>
            <tr>
              <th>Code</th>
              <th>Meaning</th>
            </tr>
          </thead>
          <tbody>
            {m.gaps.map((g) => (
              <tr key={g.code}>
                <td className="mono">{g.code}</td>
                <td>{g.message}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      <Card title="Verification">
        {v.state === "unavailable" ? (
          <p className="muted">
            Not checked: {v.reason}. Run <code>miaudit verify {d.audit_id}</code>.
          </p>
        ) : (
          <>
            <table>
              <thead>
                <tr>
                  <th>Check</th>
                  <th>Result</th>
                  <th>Detail</th>
                </tr>
              </thead>
              <tbody>
                {v.checks.map((c) => (
                  <tr key={c.check}>
                    <td>{c.check}</td>
                    <td>
                      <Badge kind={c.result === "pass" ? "ok" : c.result === "partial" ? "warn" : "bad"}>{c.result}</Badge>
                    </td>
                    <td className="mono">{c.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="muted small">Digests only, unsigned. Claims in context, decisions and evidence are not verified.</p>
          </>
        )}
      </Card>
    </>
  );
}

export function AuditPage() {
  const { owner = "", name = "", id = "" } = useParams();
  const slug = `${owner}/${name}`;
  const { data, error } = useAsync(() => api.audit(slug, id), [slug, id]);
  if (error) return <Message title="Cannot load audit commit" text={error} />;
  if (!data) return <p className="muted">Loading…</p>;
  return <AuditView slug={slug} d={data} />;
}
