import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import type { GitLink, Provenance } from "./types";
import { safeGithubUrl } from "./util";

export function Badge({ kind, children }: { kind: "ok" | "warn" | "bad" | "neu"; children: ReactNode }) {
  return <span className={`badge b-${kind}`}>{children}</span>;
}

const PROV: Record<Provenance, ["ok" | "warn" | "bad" | "neu", string]> = {
  observed: ["ok", "observed"],
  self_reported: ["warn", "self-reported"],
  derived: ["neu", "derived"],
  unavailable: ["bad", "unavailable"],
  redacted: ["neu", "redacted"],
};

export function ProvBadge({ p }: { p: Provenance }) {
  const [kind, label] = PROV[p] ?? ["neu", String(p)];
  return <Badge kind={kind}>{label}</Badge>;
}

const CORR: Record<GitLink["correspondence"], ["ok" | "warn" | "bad", string]> = {
  full: ["ok", "full match"],
  partial: ["warn", "partial match"],
  no_match: ["bad", "no match"],
  asserted: ["warn", "asserted, unverified"],
};

export function CorrBadge({ c }: { c: GitLink["correspondence"] }) {
  const [kind, label] = CORR[c] ?? ["warn", String(c)];
  return <Badge kind={kind}>{label}</Badge>;
}

export function GithubLink({ url, children }: { url: string | null | undefined; children: ReactNode }) {
  const safe = safeGithubUrl(url);
  return safe ? (
    <a href={safe} rel="noopener noreferrer" target="_blank">
      {children}
    </a>
  ) : (
    <>{children}</>
  );
}

export function GitChips({ git }: { git: GitLink[] }) {
  if (git.length === 0) return <span className="muted">not linked to a Git commit</span>;
  return (
    <>
      {git.map((g) => (
        <span key={`${g.commit}${g.pull_request?.number ?? ""}`} className="nowrap">
          <code>
            <GithubLink url={g.url}>{g.commit.slice(0, 10)}</GithubLink>
          </code>
          {g.pull_request && (
            <>
              {" "}
              <GithubLink url={g.pull_request.web_url}>{`PR #${g.pull_request.number}`}</GithubLink>
            </>
          )}{" "}
          <CorrBadge c={g.correspondence} />{" "}
        </span>
      ))}
    </>
  );
}

export function Kv({ k, children }: { k: string; children: ReactNode }) {
  return (
    <>
      <div className="k">{k}</div>
      <div>{children}</div>
    </>
  );
}

export function BulletList({ items }: { items: string[] }) {
  if (items.length === 0) return <span className="muted">none</span>;
  return (
    <ul>
      {items.map((i, n) => (
        <li key={`${n}-${i}`}>{i}</li>
      ))}
    </ul>
  );
}

export function Message({ title, text }: { title: string; text: string }) {
  return (
    <div className="card">
      <h2>{title}</h2>
      <p>{text}</p>
      <p>
        <Link to="/">← All repositories</Link>
      </p>
    </div>
  );
}
