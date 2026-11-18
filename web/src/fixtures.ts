import type { AuditDetail } from "./types";

export const XSS = "<script>alert(1)</script> & \"quotes\"";
export const AUDIT_ID = `audit_${"a".repeat(64)}`;
export const EVIDENCE_ID = `evd_${"b".repeat(64)}`;
export const SNAPSHOT_ID = `snapshot_${"c".repeat(64)}`;

export function detail(overrides: Partial<AuditDetail> = {}): AuditDetail {
  return {
    audit_id: AUDIT_ID,
    manifest: {
      repo_id: `repo_${"d".repeat(32)}`,
      parents: [],
      description: XSS,
      created: { at: "2026-01-01T00:00:00Z", clock_authority: "local_device_unverified" },
      producer: { tool: "miaudit-cli", version: "0.1.0" },
      author: { github_login: "test-user", github_id: 1, source: "gh_api_user", provenance: "derived", verification: "gh_authenticated_session_unsigned" },
      snapshot: {
        id: SNAPSHOT_ID,
        provenance: "self_reported",
        verification: "unverified_client_snapshot",
        scope: "staged_paths_only",
        git_baseline: { state: "unavailable", reason: "none" },
        exclusions: [".git/ (any depth)"],
        changes: [{ path: "src/a.rs", operation: "modify", digest_after: `sha256:${"e".repeat(64)}`, mode_after: 0o100644 }],
        entries_count: 3,
      },
      coverage: [{ capability: "file_snapshot", provenance: "self_reported", note: "captured by the local CLI" }],
      gaps: [{ code: "NO_HUMAN_APPROVAL", message: "no human disposition recorded", subjects: [] }],
    },
    bindings: [
      {
        binding_id: `bind_${"f".repeat(64)}`,
        body: {
          audit_id: AUDIT_ID,
          git: { commit: "0123456789abcdef0123456789abcdef01234567", algorithm: "sha1", repository_web_url: "https://github.com/acme/repo", commit_web_url: "https://github.com/acme/repo/commit/0123456789abcdef" },
          correspondence: "full",
          verification: "local_git_inspection_unsigned",
          provenance: "derived",
          paths: [{ path: "src/a.rs", operation: "modify", state: "match" }],
          pull_request: { number: 7, web_url: "https://github.com/acme/repo/pull/7", head_commit: "0123456789abcdef", state: "OPEN" },
        },
      },
    ],
    contexts: [{ id: `ctx_${"1".repeat(64)}`, object: { kind: "context", schema_version: "1", provenance: "self_reported", body: { description: XSS, task_ref: null, constraints: ["no comments"], source_refs: ["policy://x"] } } }],
    decisions: [
      {
        id: `dec_${"2".repeat(64)}`,
        object: { kind: "decision", schema_version: "1", provenance: "self_reported", body: { question: "q?", choice: "c", alternatives: ["x"], rationale: "r", risks: [], snapshot_id: SNAPSHOT_ID, evidence_links: [] } },
      },
    ],
    evidence: [
      {
        id: EVIDENCE_ID,
        object: { kind: "evidence", schema_version: "1", provenance: "self_reported", body: { classification: "internal", description: "after screenshot", artifact: { digest: `sha256:${"9".repeat(64)}`, size: 12 }, decision_id: null, phase: "after", environment: "local" } },
        preview: null,
        artifact_state: "present (digest checked when stored)",
        image: true,
      },
      { id: `evd_${"8".repeat(64)}`, object: null, preview: null, artifact_state: "object unavailable", image: false },
    ],
    verification: { state: "result", integrity: "pass", checks: [{ check: "tree_digest", result: "pass", detail: "ok" }] },
    ...overrides,
  };
}
