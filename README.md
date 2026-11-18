# auditAI

**A system of record for AI-assisted software decisions, human approvals, and production provenance.**

Status: product and architecture proposal. No implementation exists. Rust is the intended implementation language. The name is provisional and has not been checked for trademark or domain availability.

Detailed design: [Evidence MCP and execution harness](docs/evidence-mcp-and-harness.md). This defines how agents submit structured decisions and how a Rust harness independently captures execution evidence.

The next proposed delivery surface is a CLI and a normalized API, with no UI implementation: [CLI, audit commits and API contract](docs/cli-and-normalized-api.md). The CLI and MCP share typed domain operations; AI statements remain distinct from verified observations.

Detailed interaction capture: [Prompts, responses, authorizations and review evidence](docs/interaction-and-review-evidence.md). This links the conversation to command permissions, actual execution, file modifications and observable developer verification actions.

## 1. Product thesis

Banks, fintechs, and teams building critical software need to explain how a production change came into existence. A Git diff captures the result, but usually omits the instructions, source material, tool access, agent delegation, rejected alternatives, and human approvals that produced it.

auditAI would provide a GitHub-like interface for this evidence. Every change would have an audit dossier linking agent activity to code, review, build artifacts, and deployment. Teams could answer:

- Who requested the change, and what constraints did they give the AI?
- Which agent sessions contributed, including delegated agents?
- Which documentation, files, datasets, and MCP tools were observed being accessed?
- What recommendations and explanations were presented to the developer?
- Which decisions did a human explicitly accept, reject, or modify?
- Which exact revision was reviewed, built, and deployed?
- Where is evidence missing, self-reported, or contradicted?

The initial product integrates with existing Git hosting rather than replacing it. Its own repository pages, diffs, timelines, review queues, and release dossiers supply the GitHub-like experience.

## 2. Boundaries of the claim

auditAI records observable actions and explicit decision records. It does not extract private model reasoning or claim to reconstruct an agent's internal thought process. An agent's explanation is attributed testimony, not independent proof of why it generated code.

Observed file access does not prove that a model understood or used the file. A visible response does not prove that a developer read it. Tool permission does not prove actual access. A commit author is not automatically the requester, reviewer, or production approver.

The product establishes evidence of actions and assigned organizational responsibility. It does not automatically determine legal liability, prove software correctness, or guarantee regulatory compliance. Missing telemetry must remain visible rather than becoming an implied clean audit.

## 3. Primary users

| User | Main workflow |
| --- | --- |
| Developer | Start an instrumented session, inspect collected evidence, explain decisions, submit a change |
| Reviewer | Review code alongside agent inputs, access history, unresolved risks, and evidence gaps |
| Security and compliance | Define controls, investigate sensitive access, export evidence for an audit |
| Release approver | Authorize a specific artifact and destination environment |
| Incident responder | Trace deployed behavior back to code, sessions, approvals, and source material |
| Platform administrator | Operate collectors, identity, storage, retention, and integration policies |

## 4. End-to-end workflow

1. **Enroll a repository.** Connect Git hosting, enterprise identity, CI, and deployment systems. Select a versioned policy and data retention profile.
2. **Start a session.** A local collector establishes developer identity, device identity, workspace, repository, base revision, agent client/version, and model identifier where available. It records active capture capabilities and gaps.
3. **Capture context and actions.** Supported adapters record developer messages, visible agent outputs, tool calls, file reads and writes, documentation references, MCP access, command results, and delegation relationships. Sensitive content is filtered before upload.
4. **Record decisions.** The agent or developer proposes a structured decision containing alternatives, a concise rationale, supporting evidence, affected code, and risks. Developers explicitly accept, amend, reject, or defer material decisions.
5. **Bind evidence to a commit.** Capture the final tree and patch digest, parent revision, working changes, sessions, and policy version. Register an immutable dossier referencing the resulting commit SHA. Store this externally to avoid a circular dependency between commit contents and its own SHA.
6. **Review the pull request.** auditAI presents the code diff, decision records, evidence coverage, sensitive access, and policy results. An approval binds the approver's identity and role to an exact revision and dossier digest.
7. **Re-evaluate changes.** New code, new material evidence, or a policy-relevant correction creates another dossier revision and invalidates affected approvals. Policies define when fresh review is mandatory.
8. **Bind the build and release.** CI attests to source revision, build inputs, test results, and artifact digest. Deployment records bind that digest to an environment, release approval, and time.
9. **Investigate or export.** An auditor opens a production release and follows verified links back to its decisions and evidence. Exports include a signed manifest, policy evaluations, redactions, and verification instructions.

Example: an agent changes a transfer limit after reading an internal risk policy through MCP. The developer rejects its proposed default, selects a lower limit, and explains why. A separate reviewer approves the resulting revision. The release dossier shows the policy document version, observed MCP access, original proposal, human amendment, passing checks, approver, and deployed artifact. If the document contents could not be captured, the dossier explicitly says what reference or digest remains available.

## 5. What is captured

| Category | Evidence to retain |
| --- | --- |
| Human context | Request text or redacted reference, constraints, attachments, corrections, identity, time |
| Agent context | Client/version, reported model/version, system or project instructions when exposed, configuration and capability manifest |
| Agent topology | Session and agent IDs, parent/child links, delegation purpose, tool permissions, start/end state |
| Documentation | URI/path, observed version, content digest, capture time, permitted snapshot or excerpt |
| Data access | Source, classification, operation, scope, resource identifier, outcome, redacted arguments and response references |
| MCP | Server identity and version when available, tool/schema version, authenticated principal, invocation, result status and evidence references |
| Local work | Repository/worktree identity, base tree, uncommitted input changes, patches, commands, exit status, output digests |
| Recommendations | Output presented by the client where observable; otherwise output emitted, with presentation unverified |
| Decisions | Proposal, alternatives, stated rationale, evidence links, disposition, actor and affected revision |
| Verification | Check invocation, environment, result, logs or references, and which revision was checked |
| Delivery | Commit/PR relationships, review events, build provenance, artifact digest, deployment and rollback events |

Every category supports `observed`, `self-reported`, `derived`, `unavailable`, and `redacted` provenance labels as appropriate. Evidence coverage is reported per capability and interval; a single score must not conceal missing critical sources.

## 6. Domain model

All records are tenant-scoped. Stable identifiers are separate from Git SHAs and external provider identifiers.

| Entity | Purpose and relationships |
| --- | --- |
| Organization / Project / Repository | Ownership, access boundaries, repository identity and policies |
| Principal | Human, agent, service, or device identity; issuer and authentication provenance |
| Session / AgentRun | One work session and its agent execution graph; many sessions may contribute to one commit |
| ContextItem / ResourceAccess | Versioned context references and observed accesses, including classification and redaction |
| EvidenceEvent | Immutable observation with producer, sequence, timestamp, provenance and artifact references |
| EvidenceArtifact | Encrypted content or redacted snapshot, digest, storage reference and retention class |
| Decision | Versioned proposal and explicit dispositions; supporting events and affected code |
| ChangeSet / CommitBinding | Captured code change and its verified or asserted links to Git revisions |
| AuditDossier | Immutable manifest assembling evidence for an exact revision; supersession links |
| Approval | Attestation by a principal over a dossier/revision, role, scope and policy version |
| Policy / PolicyEvaluation | Versioned rules and reproducible outcomes with reasons |
| Build / Deployment | Source-to-artifact and artifact-to-environment links |
| Exception | Authorized, scoped, expiring policy bypass with reason and follow-up owner |

An event envelope would include a schema version, event ID, tenant/repository/session IDs, producer ID and sequence, event type, occurrence time, ingestion time, actor, parent/causal references, payload digest, artifact references, provenance label, redaction metadata, and producer signature where supported. Device clock time is not authoritative; receipt time and sequence provide additional ordering evidence.

Decision lifecycle: `proposed → accepted | amended | rejected | deferred`, followed by explicit supersession where necessary. Each transition is a new event. Agent-generated summaries must retain links to original observations and identify their generator/version.

## 7. Human responsibility and approvals

Keep distinct roles: requester, session operator, decision owner, code author, reviewer, risk owner, and release approver. One person may hold multiple roles when policy permits; critical changes may require separation of duties.

An acceptance must identify what the person accepted: a recommendation, a code revision, a documented risk, or a production release. Store identity, authentication method, server receipt time, statement, scope, and evidence digest. Never infer acceptance merely from continuing a chat, creating a commit, or clicking a generic tool permission prompt.

Repository ownership defines responsibility assignments prospectively. auditAI records assignments and actions without generating developer blame scores. Break-glass releases require a named authorized actor, reason, scope, expiry, and retrospective review. Delegation does not silently transfer an approver's responsibility.

## 8. Integration strategy

Claude Code, Codex, and Grok are target integrations, not claims of currently available equivalent instrumentation. Before implementation, validate each client's documented extension points, supported versions, export formats, terms, and available telemetry.

| Integration | Proposed approach | Known design constraint |
| --- | --- | --- |
| Claude Code | Adapter using supported hooks or session exports, supplemented by a managed launcher where needed | Coverage depends on documented client features and version |
| Codex | Adapter using supported execution events, hooks or exports where available | Hosted and local execution may expose different evidence |
| Grok | Adapter for the specific coding client or an organization-controlled API gateway | A model API alone cannot observe developer UI, local files, or external tools |
| MCP | Instrument an organization-controlled gateway or participating MCP servers | Direct or unmanaged connections may bypass capture |
| Git hosting | App/webhooks for repositories, PRs, reviews and commit discovery | Webhook signatures, deduplication and reconciliation are required |
| CI/CD | Signed provenance and deployment ingestion; policy checks on exact revisions | CI identity and protected deployment permissions must be enforced externally |

Each adapter declares a versioned capability manifest. Unsupported capture produces an evidence gap and a policy outcome. Manual imports are allowed but labeled as imports; they cannot retroactively establish trustworthy live capture.

Launch with one validated client, Git provider, and CI path, then expand using a shared event schema. The first implementation milestone is a capability investigation and integration contract, before committing to automatic capture promises.

## 9. Git provenance and difficult cases

- **Several agents or developers:** preserve the contribution graph and distinct principals; do not assign every line to the session operator.
- **Mixed manual and generated changes:** record contributions as observed or asserted. Patch overlap alone is insufficient proof of authorship.
- **Squash, rebase and cherry-pick:** retain original bindings and add transformation records. Verify resulting trees/diffs; record uncertainty and require fresh approvals where policy demands it.
- **Uncommitted inputs:** record relevant dirty workspace state, including files that influenced generation but never entered Git, subject to content policy.
- **Offline sessions:** spool encrypted events locally with sequences and signatures. Show delayed ingestion and missing intervals; policies may prohibit release until synchronization completes.
- **Bypassed collectors:** detect absence of expected attestations through protected CI/release gates. Do not claim that a local collector can prove no unobserved activity occurred.
- **Rollback and emergency fixes:** create new deployment records and explicit exceptions rather than rewriting prior evidence.
- **Retries and disorder:** use idempotent event IDs, producer sequences and causal links; preserve late events and issue updated dossier evaluations.

## 10. Architecture in Rust

Proposed logical components, initially deployed as a modular server plus local collector rather than many microservices:

```text
Agent clients / developer workspace / MCP gateway
                    |
            Rust collector + adapters
       filtering, signing, encrypted local spool
                    |
             Rust ingestion API <--- Git / CI / deployment integrations
                    |
       immutable event store + encrypted artifact storage
                    |
       provenance assembler + policy evaluator
                    |
       query API ---> repository UI / audit exports / release checks
```

Candidate Rust modules: `audit-schema`, `audit-harness`, `audit-mcp`, `audit-collector`, `audit-adapters`, `audit-ingest`, `audit-provenance`, `audit-policy`, `audit-api`, and `audit-export`. The harness supervises execution and the collector records observations; the MCP server exposes scoped evidence tools to agents. These are proposed boundaries, not an instruction to scaffold code.

Candidates to evaluate: Tokio for asynchronous execution, Axum for HTTP APIs, Serde for versioned event serialization, SQLx with PostgreSQL for metadata, and S3-compatible object storage for evidence artifacts. A Rust UI framework such as Leptos is an option if the web application must also be Rust; that choice remains open.

Use transactional ingestion and an outbox for reliable downstream processing. PostgreSQL read models support repository timelines and investigations; object storage holds larger encrypted evidence. Search indexes are derived, rebuildable, and subject to the same tenant authorization and deletion rules. Start with database-backed jobs; introduce a separate broker only if load requires it.

Support private-cloud or on-premises deployment for regulated customers, with customer-controlled keys, regional storage, private networking and optional restricted egress. Define availability, recovery objectives, event volume and artifact sizes during pilot discovery before selecting a topology.

## 11. Evidence integrity and threat model

Threats include developer tampering, compromised collectors, forged identities, malicious agents, untrusted MCP outputs, administrator misuse, cross-tenant access, and evidence loss.

Collectors sign event batches with enrolled device identities. Ingestion acknowledges accepted sequence ranges. Per-producer hash chains and signed server checkpoints expose modification or missing segments relative to a trusted checkpoint. Store checkpoints independently and support retention-locked object storage. Signatures establish provenance of a statement, not its truth; a compromised producer can still emit false evidence.

Use enterprise identity for humans, short-lived credentials for services, separate signing keys, key rotation/revocation history, and independently audited administrative access. Verify external webhook signatures and reconcile periodically with source systems.

Treat all captured prompts, documents and tool results as untrusted data. Escape rendered content, avoid executable attachments, and isolate any summarization agent from privileged actions. An instruction inside captured evidence must never alter policy evaluation.

Export verification checks manifest signatures, artifact digests, chains/checkpoints, key validity history and redaction declarations. It must identify unavailable artifacts and unverifiable links explicitly. Integrity proofs cannot prove collection completeness; managed environments and independent CI observations strengthen that assurance.

## 12. Sensitive data and retention

The evidence store itself is highly sensitive: prompts and tool results may contain customer data, credentials, proprietary code or financial records.

Default to minimal collection with source classification, local redaction, secret filtering, and configurable metadata-only capture. Record that filtering occurred and which policy version applied. Do not assume content hashes anonymize low-entropy sensitive values; use keyed digests or omit them where appropriate.

Separate broadly visible metadata from restricted evidence payloads. Apply tenant isolation, repository authorization, field/artifact-level access, encryption in transit and at rest, and audited evidence viewing and export. Restricted artifacts must not leak through search, generated summaries, or cached previews.

Define retention by evidence class and jurisdiction. Legal holds suspend eligible deletion. Payload deletion or cryptographic erasure creates a tombstone event while preserving only legally permitted metadata; backups and indexes must follow documented expiry procedures. This means append-only history does not imply permanent retention of every captured payload. Counsel and security teams must validate the retention policy for each deployment.

Do not collect passwords, API tokens, or unrestricted customer datasets merely to improve audit completeness. A scoped reference, access receipt, and approved document version may be the appropriate evidence.

## 13. Policy model

Policies are versioned, deterministic rules evaluated against a dossier and its evidence coverage. Initial outcomes: `pass`, `fail`, `insufficient_evidence`, and `exception_authorized`. Preserve the original evaluation and any later re-evaluation separately.

Examples for a high-risk payments repository:

- A production change requires authenticated requester and reviewer identities.
- Changes to settlement logic require a separate domain reviewer and explicit decision acceptance.
- Access to a restricted source requires a recorded authorization and approved access path.
- Critical missing telemetry blocks release until evidence is reconciled or an authorized exception exists.
- Approval must match the final reviewed revision; deployment must match the approved artifact digest.
- Failed required checks prevent release; a summary saying “tests passed” is insufficient.

auditAI publishes checks, but Git branch protection and deployment infrastructure enforce them. Policy changes and exceptions are themselves reviewed and audited. A green check means specified controls passed against available evidence, not that the code is secure or correct.

## 14. Product experience

- **Organization overview:** repositories, pending reviews, evidence gaps, expiring exceptions and recent deployments.
- **Repository page:** code-linked audit history, contributors and role assignments, policies and integration health.
- **Commit dossier:** diff beside decisions, context, sources, agent graph, tool activity and approvals.
- **Decision review:** accept, amend, reject or defer with explicit scope and supporting evidence.
- **Data access view:** permission grants separated from observed resource access, with restricted payload controls.
- **Release page:** source → dossier → review → build → artifact → environment, with broken links highlighted.
- **Audit export:** bounded time/repository/release scope, signed manifest and independently verifiable evidence bundle.

The default view should explain the change in a few minutes. Raw events remain accessible to authorized investigators. Generated summaries are optional navigation aids and never replace original evidence.

## 15. MVP and later scope

**Discovery:** validate one client's actual capture surface, interview a fintech engineering/security team, classify data, and define the minimum acceptable release dossier and retention policy.

**MVP:** single-organization deployment with tenant-scoped records, one agent adapter, one Git integration, collector, event/artifact ingestion, commit dossiers, explicit human decisions, revision-bound approvals, basic policy checks, one CI/deployment integration, and signed audit exports. Include evidence gaps, redaction and access controls from the start.

**Next:** additional agent clients, managed MCP capture, multi-agent contribution graphs, stronger enterprise identity, legal holds, private installation tooling and more CI providers.

**Later:** cross-repository incident analysis, reusable control mappings and organization-wide evidence queries.

Out of initial scope: replacing Git hosting, generating application code, autonomous remediation, employee productivity scoring, legal liability determination, universal visibility into arbitrary agents, and certification of regulatory compliance.

## 16. Validation criteria

Before production use, demonstrate that:

1. A deployed artifact can be traced to the exact reviewed revision, dossier and responsible approvers.
2. A subsequent code change cannot reuse an obsolete revision-bound approval.
3. A missing collector interval appears as insufficient evidence under a policy requiring that capture.
4. Altering a retained evidence artifact fails independent verification.
5. A redacted secret never appears in uploaded payloads, derived summaries, indexes or exports in the tested capture paths.
6. An unauthorized principal cannot retrieve another repository's or tenant's restricted evidence.
7. Squashed and rebased changes preserve traceable relationships without overstating authorship.
8. Offline retries do not duplicate records; gaps and delayed arrivals remain visible.
9. Retention deletion removes eligible payloads and derived copies while preserving permitted tombstones and honoring holds.
10. An emergency release leaves an explicit exception and required follow-up review.

Pilot measures: time to reconstruct a release, proportion of releases with complete required provenance, unresolved gap rate, reviewer effort, collector overhead, evidence storage cost, and false policy-block rate. Set numerical targets with pilot customers.

## 17. Decisions still open

- Which client/version offers sufficient supported capture for the first adapter?
- Does “Grok” mean a particular coding client, API-based agent, or another workflow?
- Which Git host, CI platform and deployment environment should the first pilot support?
- Which events require managed workstations or gateways for the desired assurance level?
- Which jurisdictions, retention rules and customer data classifications apply?
- Should the first deployment be customer-hosted only or also offer managed hosting?
- Which actions require stronger authentication and separation of duties?
- How much source and conversation content may leave a developer workstation?
- Is an entirely Rust web stack required, or only the collector and backend?

These questions guide discovery. They do not change the core model: observable evidence, explicit human decisions, and verified links from development activity to production.
