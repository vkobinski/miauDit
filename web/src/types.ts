export type Provenance = "observed" | "self_reported" | "derived" | "unavailable" | "redacted";

export interface Stored<T> {
  kind: string;
  schema_version: string;
  provenance: Provenance;
  body: T;
}

export interface RepoSummary {
  slug: string;
  repo_id: string;
  remote_url: string | null;
  audits: number;
  gaps: number;
  last_audit_at: string | null;
  last_push_at: string | null;
}

export interface RepoInfo {
  repo: { slug: string; repo_id: string; remote_url: string | null; last_push_at: string | null };
  authors: { login: string; count: number }[];
}

export interface PullRequestRef {
  number: number;
  web_url: string;
  head_commit: string;
  state: string;
}

export interface GitLink {
  commit: string;
  correspondence: "full" | "partial" | "no_match" | "asserted";
  url: string | null;
  pull_request: PullRequestRef | null;
}

export interface AuditRow {
  audit_id: string;
  created_at: string;
  description: string;
  author: string | null;
  changes: number;
  gaps: number;
  gap_codes: string[];
  parents: string[];
  git: GitLink[];
}

export interface AuditPage {
  repo: string;
  total: number;
  page: number;
  per_page: number;
  audits: AuditRow[];
}

export interface Author {
  github_login: string | null;
  github_id: number | null;
  source: string;
  provenance: Provenance;
  verification: string;
}

export interface Change {
  path: string;
  operation: "add" | "modify" | "delete" | "type_change";
  digest_after: string | null;
  mode_after: number | null;
}

export type GitBaseline = { state: "known"; commit: string; algorithm: string } | { state: "unavailable"; reason: string };

export interface Manifest {
  repo_id: string;
  parents: string[];
  description: string;
  created: { at: string; clock_authority: string };
  producer: { tool: string; version: string };
  author?: Author;
  snapshot: {
    id: string;
    provenance: Provenance;
    verification: string;
    scope: string;
    git_baseline: GitBaseline;
    exclusions: string[];
    changes: Change[];
    entries_count: number;
  };
  coverage: { capability: string; provenance: Provenance; note?: string }[];
  gaps: { code: string; message: string; subjects: string[] }[];
}

export interface PathCheck {
  path: string;
  operation: Change["operation"];
  state: "match" | "content_differs" | "mode_differs" | "absent_in_git" | "present_in_git";
}

export interface Binding {
  binding_id: string;
  body: {
    audit_id: string;
    git: { commit: string; algorithm: string; repository_web_url: string | null; commit_web_url: string | null };
    correspondence: GitLink["correspondence"];
    verification: string;
    provenance: Provenance;
    paths: PathCheck[];
    pull_request?: PullRequestRef;
  };
}

export interface ContextBody {
  description: string;
  task_ref: string | null;
  constraints: string[];
  source_refs: string[];
  change_kind?: string;
}

export interface DecisionBody {
  question: string;
  choice: string;
  alternatives: string[];
  rationale: string;
  risks: string[];
  snapshot_id: string;
  evidence_links: { evidence_id: string; relation: string }[];
  supersedes?: string[];
}

export interface EvidenceBody {
  classification: string;
  description: string;
  artifact: { digest: string; size: number };
  decision_id: string | null;
  phase?: string;
  environment?: string;
}

export interface Item<T> {
  id: string;
  object: Stored<T> | null;
}

export interface DecisionItem extends Item<DecisionBody> {
  superseded_by: { decision_id: string; audit_id: string }[];
}

export interface EvidenceItem extends Item<EvidenceBody> {
  preview: string | null;
  artifact_state: string;
  image: boolean;
}

export type Verification =
  | { state: "result"; integrity: string; checks: { check: string; result: string; detail: string }[] }
  | { state: "unavailable"; reason: string };

export interface AuditDetail {
  audit_id: string;
  manifest: Manifest;
  bindings: Binding[];
  contexts: Item<ContextBody>[];
  decisions: DecisionItem[];
  evidence: EvidenceItem[];
  verification: Verification;
}

export interface AppConfig {
  mode: "server" | "local";
  repo: string | null;
}
