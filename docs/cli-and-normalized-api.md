# CLI, audit commits and normalized API

Status: design contract. Part of it is implemented (the local CLI, a JSON read API, a push endpoint and a viewer; see [crates/README.md](../crates/README.md)), and this document describes the intended whole, so command names and shapes here can differ from what exists. Extends the [product model](product-proposal.md) and [MCP/harness contract](evidence-mcp-and-harness.md). The immediate delivery scope is Rust services, CLI and MCP, without a graphical interface. Earlier UI concepts remain possible later consumers.

## 1. Shared interface

Expose one versioned, typed domain interface through an HTTP JSON API. The Git-like `audit` CLI and miauDit MCP server are adapters to that interface. They must not have different validation, authorization, evidence or approval rules.

The API accepts structured records instead of treating a chat transcript as an authoritative audit. Free text remains useful for context descriptions and concise rationales, but identity, source references, file state, check outcomes and approvals have explicit types and verification requirements.

Normalization reduces ambiguity and prevents unsupported transitions. It cannot prove a narrative is true. A structurally valid statement may still be false; origin, verification status and evidence coverage remain visible.

## 2. Git-like CLI

Proposed commands, not executable examples of an existing implementation:

| Command | Meaning |
| --- | --- |
| `miaudit init` | Enroll repository/worktree and create local audit metadata |
| `miaudit status` | Show changed files, staged snapshot, pending evidence, gaps and synchronization status |
| `miaudit context add --file context.json` | Validate and append structured task context with a human-readable description |
| `miaudit add <paths>` | Capture and stage file states and their content hashes |
| `miaudit diff` | Compare workspace with staged snapshot, or staged snapshot with the last audit commit |
| `miaudit decision add --file decision.json` | Append a typed proposal, alternatives, rationale, risks and evidence references |
| `miaudit evidence add --file evidence.json` | Submit attributed evidence under the caller's allowed producer type |
| `miaudit check <check-id>` | Request a registered harness check against an immutable staged snapshot |
| `miaudit gaps` | Explain missing fields, unsupported capture and policy requirements |
| `miaudit commit -m <description>` | Create an immutable audit commit from staged file state and selected evidence |
| `miaudit bind <audit-id> --git-commit <sha>` | Request verified linkage to an actual Git revision |
| `miaudit log` / `miaudit show <audit-id>` | Inspect audit history and a specific manifest |
| `miaudit verify <audit-id>` | Verify available digests, signatures, references and coverage requirements |
| `miaudit push` / `miaudit fetch` | Synchronize authorized immutable audit objects and server receipts |
| `miaudit review request <audit-id>` | Request human disposition over an exact manifest |
| `miaudit approve <audit-id> --scope <scope>` | Record an explicit disposition through separate human authentication |

Machine output uses versioned JSON, with stable exit codes for validation failure, conflict, insufficient evidence, authorization failure and transport failure. Interactive text is an alternate rendering of the same response. Secrets must not be supplied in command arguments.

`miaudit commit` creates an audit object, not a Git commit, and does not imply policy success or approval. A policy requiring complete evidence can block release while allowing an incomplete audit commit to preserve the history. `miaudit push` confirms storage only after server acknowledgement. Local history can be inspected offline; restricted operations and release gates require authenticated verification.

## 3. Files, staging and hashes

An audit index captures exact staged bytes and file metadata. Later workspace edits do not mutate the index: `miaudit status` shows divergence, and checks use the indexed snapshot. Each snapshot identifies the Git baseline and the complete auditable tree or explicit exclusions, not just a list of interesting paths.

Each changed entry records repository-relative path, operation (`add`, `modify`, `delete`, `type_change`), object kind, file mode, before and after content digests where applicable, size, and artifact availability. Deletions have no after digest. Renames are optional asserted relationships between deletion and addition; content similarity alone does not establish intent.

Use versioned SHA-256 content identifiers over exact bytes. Git object IDs remain separate identifiers with their algorithm stated. A tree digest covers sorted path entries, kinds, modes and content references using canonical serialization. It must detect permission/type changes even when file contents remain identical.

Preserve line endings and byte identity. Symlinks hash their target bytes without following them. Submodules identify the referenced repository revision; Git LFS pointers and fetched payloads are separate artifacts. Reject traversal and ambiguous path encodings; initially define supported UTF-8 paths explicitly rather than silently normalizing distinct filesystem names. Record case sensitivity constraints.

Hashing must operate on a stable captured copy or validate that the source did not change during capture; otherwise retry or mark capture indeterminate. Ignore rules, excluded files and incomplete snapshots are recorded. Missing content is not an empty file. Local storage containing source bytes is encrypted and governed by retention policy.

Sensitive evidence may require keyed digests or metadata-only capture. Such evidence declares its digest scheme and verification limits; it must not masquerade as a publicly reproducible content hash. Redaction creates a distinct artifact with its own digest and transformation metadata.

## 4. Immutable audit commit

An audit commit includes:

- Schema and canonicalization versions, repository identity and parent audit commit IDs.
- File snapshot/tree digest and changed-file entries.
- Task/context description, constraints, captured developer messages or restricted references.
- Agent runs, parent/child relationships, model/client identifiers when available, and coverage manifests.
- Documentation and data accesses, MCP servers/tools, permissions versus observed accesses, and source versions.
- Proposed decisions, alternatives, stated rationale, risks, and supersession links.
- Agent outputs and presentation observations where supported.
- Check receipts, evidence artifacts, gaps and policy version/evaluation references.
- Producer attribution and timestamps with their clock authority stated.

The audit ID is a domain-separated SHA-256 digest of the canonical manifest excluding its own ID and signatures. Signatures, server receipts, Git bindings and later human approvals are separate attestations referencing that ID; this avoids circular hashing. An approval cannot be silently added to an already hashed manifest.

Corrections create new objects and supersession events. Branches and concurrent work form a graph with explicit parents; there is no requirement for one global linear history. Merge manifests preserve contributing records and unresolved contradictions. A Git commit may have several audit records, and an audit record may have verified transformation bindings to multiple Git revisions.

## 5. Normalized input model

Publish OpenAPI and JSON Schema contracts generated from or checked against shared Rust types. Pin schema versions and reject unsupported versions. Use closed object schemas, explicit enums, bounded strings/arrays, typed digests and IDs, and discriminated unions rather than arbitrary metadata maps. Extensions require a registered namespace and schema.

| Input type | Required structure | Validation boundary |
| --- | --- | --- |
| Context | Description, task reference, constraints, source references | Narrative is attributed; source existence and authorization are verified |
| Decision | Question, choice, alternatives, concise rationale, risks, affected snapshot, evidence links | Proposal only; no agent-supplied human disposition |
| Evidence submission | Artifact reference or bounded content, classification, relation | Origin derived from authenticated producer, not supplied by AI |
| Access claim | Enrolled source reference, claimed operation, related decision | Remains claimed unless corroborated by a permitted observer |
| Execution request | Registered check, typed parameters, expected snapshot | Broker determines actual execution status and receipt |
| Commit request | Snapshot, parent audit IDs, context/decision/evidence IDs, description | Server verifies reference scope and immutable object integrity |
| Human disposition | Exact audit/decision ID, scope, explicit disposition and statement | Separate human authentication, role checks and stale-content preconditions |

Unknown, unavailable and not applicable are explicit states with reason codes where allowed. Do not substitute invented values, empty strings or model confidence scores for missing facts. “Not applicable” is checked against policy; an agent cannot use it to waive a required control.

Example proposal input (illustrative identifiers):

```json
{
  "schema_version": "1",
  "question": "Which transfer limit should apply?",
  "choice": "Use the configured account risk limit",
  "alternatives": ["Use one global limit"],
  "rationale": "The referenced policy specifies account-specific limits.",
  "risks": ["Missing configuration requires an explicit fallback"],
  "snapshot_id": "snapshot_example",
  "evidence_links": [
    {"evidence_id": "evidence_example", "relation": "supports"}
  ]
}
```

Even if the cited evidence exists, the API does not infer that it semantically supports the rationale. The relationship is agent-asserted until independently reviewed. Supplying fields such as `approved_by`, `verified: true` or `origin: harness` fails schema validation on this endpoint.

## 6. API surface

Proposed endpoints under `/v1`, scoped to the authenticated tenant and repository:

| Endpoint | Operation |
| --- | --- |
| `GET /capabilities` | Supported contract versions, tools and limits |
| `POST /sessions` | Establish authorized work session and capability references |
| `POST /contexts` / `POST /decisions` | Append attributed context and decision proposals |
| `POST /artifacts/uploads` | Issue constrained upload authorization; finalize only after filtering and digest validation |
| `POST /evidence` | Submit agent-attributed evidence |
| `POST /observations` | Ingest signed observations from separately authorized producers |
| `POST /snapshots` | Register harness snapshots or explicitly unverified client snapshots |
| `POST /checks` / `GET /operations/{id}` | Dispatch and inspect managed execution |
| `POST /audit-commits` / `GET /audit-commits/{id}` | Create or inspect immutable manifests |
| `POST /git-bindings` | Verify repository/tree correspondence before issuing binding |
| `GET /audit-commits/{id}/gaps` | Return requirements and unresolved coverage |
| `POST /review-requests` / `POST /human-dispositions` | Request review or record separately authenticated human action |
| `POST /sync/batches` | Idempotent immutable-object synchronization with per-object acknowledgements |

MCP tools map to these domain operations. CLI staging can be local, but server registration revalidates submitted hashes and provenance where content is available. A submitted hash without corresponding verifiable content is only a submitted hash. Artifact uploads have quotas, bounded size and scoped object identifiers; arbitrary remote URLs are not fetched.

Every mutation requires an idempotency key scoped to principal and operation. Identical retries return the original receipt; key reuse with different canonical content conflicts. Mutable workflow transitions use expected revision preconditions. Immutable object creation checks digests and references transactionally; a manifest is not accepted as complete while its required artifacts are unfinalized.

## 7. Validation and hallucination controls

Process each input through:

1. Authentication and scope resolution from trusted credentials.
2. Strict schema validation, size limits and supported-version checks.
3. Deterministic canonicalization without rewriting narrative meaning.
4. Reference resolution, tenant authorization and source enrollment checks.
5. Digest/signature verification where sufficient evidence is available.
6. Semantic invariants: snapshot consistency, legal state transitions, authorized producer types and role separation.
7. Policy evaluation and contradiction/gap detection.
8. Immutable persistence with a receipt declaring exactly what was accepted and verified.

Return structured errors with code, field pointer, violated rule, expected type/state and retryability. For example, `EVIDENCE_REFERENCE_UNAVAILABLE` can cover inaccessible or nonexistent references without leaking another tenant's data. `SNAPSHOT_MISMATCH`, `UNSUPPORTED_SCHEMA` and `HUMAN_AUTH_REQUIRED` have distinct meanings. Never silently coerce an invalid approval into a valid one or fabricate missing references.

Separate `accepted` (stored successfully), `verified` (specific integrity or origin checks passed), `policy_passed`, and `human_approved`. These are independent results. An unresolved claim can be accepted into history while leaving the dossier insufficient for release.

An optional AI extraction service may turn transcripts into draft structured proposals. Every extracted field retains its source span or is labeled an inference. Drafts pass the same validation and cannot create observations or approvals. Semantic truth and completeness still require independent evidence and human judgment; formatting alone does not solve hallucination.

## 8. No-UI workflow

A developer initializes the repository, starts the harness, stages modified files, records context and decisions through CLI or MCP, runs registered checks, and creates an audit commit. The API binds it to the resulting Git revision. A reviewer reads `miaudit show --json` or a text rendering and authenticates separately to approve the exact audit ID and revision scope.

Agents hold only agent credentials. Human CLI approval requires a separate interactive enterprise authentication flow and explicit confirmation of the manifest digest; an agent invoking the binary cannot become a human merely by selecting an approval subcommand. CI uses its own service identity for build attestations and release-policy queries.

Protected Git and deployment controls enforce the API's policy outcomes. No browser UI is required to create, inspect, review, synchronize or verify the initial evidence chain.

## 9. Acceptance criteria

- The same domain operation through CLI, MCP and HTTP produces equivalent validated records and authority checks.
- A one-byte change, file deletion, symlink-target change or executable-bit change alters the appropriate snapshot digest.
- Workspace changes after staging cannot change the staged manifest or reuse checks for another snapshot.
- Identical canonical manifests produce the same audit ID across supported Rust clients and the server.
- Invented evidence IDs, cross-tenant references and agent-authored approval fields are rejected.
- Valid JSON containing a false narrative remains visibly an attributed claim, never automatically verified fact.
- Partial uploads, offline records and unavailable payloads cannot produce a false complete-verification result.
- Human approvals, server acknowledgements and Git bindings reference immutable IDs without changing the original hash.
- Concurrent revisions and retry conflicts produce explicit responses rather than lost history.

Before implementation, finalize canonicalization test vectors, schema evolution rules, authentication integration, CLI configuration paths and retention limits. These decisions should preserve the central contract: AI supplies typed claims, trusted producers supply observations, and humans supply explicit scoped approvals.
