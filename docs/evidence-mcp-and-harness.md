# Evidence MCP and execution harness

Status: proposed design. The MCP server and the trusted execution harness described here are not implemented; hook capture in the CLI is a partial stand-in (see [crates/README.md](../crates/README.md)). Extends the [product model](product-proposal.md). Tool names, resource URIs and Rust module boundaries below are proposed miauDit contracts, not existing client capabilities.

## 1. Two cooperating components

The **miauDit MCP server** gives an AI a structured interface to report context, propose decisions, attach evidence, request verification, inspect missing evidence, and prepare a commit dossier.

The **miauDit harness** starts and supervises an agent's execution environment, captures supported actions automatically, and issues receipts for operations it actually observes. It supplies the execution substrate behind applicable MCP tools and independently ingests supported client events.

Together they capture both meaning and behavior: the agent explains the choice; the harness records the files accessed, commands executed, changes made, and outcomes observed. A model saying “I ran the tests” remains a claim until linked to an execution receipt. A test receipt establishes execution and its reported result, not correctness of the test suite.

The evidence MCP server is distinct from an MCP gateway. The former exposes miauDit tools; the latter observes or mediates access to other MCP servers. Connecting the evidence server alone does not intercept other tools, filesystem access, or conversation history.

## 2. Trust and authority

| Producer | May establish | Cannot establish alone |
| --- | --- | --- |
| Agent through MCP | Attributed proposals, stated rationale, claimed context, links to evidence | Human approval, actual external execution, completeness of capture |
| Harness / collector | Actions observed within its declared capture surface, execution receipts, workspace snapshots | All activity outside that surface, human understanding, model reasoning |
| External service / CI | Authenticated source events, build and verification receipts | What the developer saw in an unrelated UI |
| Human approval service | Explicit decisions authenticated through a separate human channel | That an approved change is safe or legally compliant |
| Policy evaluator | Reproducible conclusions under a specified policy and evidence set | Stronger provenance than the underlying evidence |

Identity, tenant, repository, session, producer and provenance labels are resolved by authenticated services. Agent arguments cannot override them. Statements copied from a developer message are claims about that message until linked to a captured source; they never become an approval merely because the agent labels them accepted.

Evidence records have separate dimensions: origin (`agent_statement`, `harness_observation`, `external_attestation`, `human_attestation`, `derived`), capture channel, content availability, and verification status. Redacted content can still have verified origin. A verified signature on an agent statement does not convert it into an observation.

## 3. Runtime topology

```text
Authenticated developer / approved task
                 |
          Rust miauDit harness
          |                 |
   agent client adapter     collector / workspace snapshotter
          |
      AI agent ----- miauDit MCP server ----- evidence ledger
          |                 |
          |           harness operation broker
          |                 |
          +-------- managed tools / checks / MCP gateway
                            |
                     execution receipts

Git + CI attestations ---> dossier assembler ---> policy / review UI
Human approval channel ------------------------> approval ledger
```

The MCP server may run locally over stdio under harness supervision or remotely over an authenticated MCP HTTP transport supported by the selected protocol/client version. Validate transport and authorization compatibility during discovery. A remote server does not gain local filesystem access: it dispatches authorized work to an enrolled harness through a separately authenticated channel.

Use distinct, short-lived session credentials for the agent and harness. Harness signing keys and human approval credentials are unavailable to the agent process. Shared host administration still represents a compromise boundary; stronger deployments isolate the harness and its keys from the agent's sandbox.

## 4. Agent-facing MCP contract

All mutating requests require an idempotency key. Evidence references must resolve within the caller's authorized scope. Unknown references, schema violations, cross-scope references and stale snapshot preconditions fail explicitly.

| Proposed tool | Essential input | Result and authority |
| --- | --- | --- |
| `audit_get_context` | Optional task reference | Session identity, active policy, capabilities, permitted sources and current snapshot |
| `audit_record_context` | Kind, statement or permitted artifact reference, source references | Agent-attributed context item; does not prove source access |
| `audit_propose_decision` | Question, proposed choice, alternatives, concise rationale, risks, evidence references, affected paths/snapshot | Versioned proposed decision awaiting any required human disposition |
| `audit_attach_evidence` | Bounded content or previously issued upload reference, classification, related decision | Filtered artifact receipt, digest and provenance; uploaded logs remain agent-supplied |
| `audit_read_resource` | Enrolled resource ID and version, purpose | Policy-filtered content/reference plus harness or gateway access receipt |
| `audit_run_check` | Registered check ID, allowed parameters, expected snapshot ID | Operation ID; eventual execution receipt tied to the checked snapshot |
| `audit_get_operation` | Operation ID | Pending, completed, failed, cancelled or indeterminate status and available receipts |
| `audit_link_evidence` | Decision ID, evidence IDs, relation such as supports or contradicts | Agent-asserted relationship, preserving source provenance |
| `audit_request_review` | Decision IDs, snapshot ID, review reason | Pending review reference and human UI link; no approval granted |
| `audit_get_gaps` | Snapshot or dossier reference | Missing requirements, unsupported coverage and unresolved contradictions |
| `audit_prepare_dossier` | Expected snapshot, relevant decisions and session references | Immutable candidate manifest and policy evaluation; may be incomplete |
| `audit_bind_commit` | Candidate ID, commit SHA | Binding after trusted repository verification, or an explicitly unverified result |

No agent-facing tool grants human approval, deletes evidence, changes policy, enrolls an arbitrary data source, or authorizes deployment. Corrections append superseding statements; prior statements remain visible under retention policy.

Read-only MCP resources can expose `audit://session/current`, `audit://policy/active`, and authorized `audit://evidence/{id}` records. Resource access uses the same authorization as tools. An optional “prepare audited change” MCP prompt teaches the workflow but is only guidance; enforcement resides in services and release gates.

Tool results include contract version, request ID, evidence/operation IDs, provenance, persistence status, relevant snapshot, gaps and policy outcome. A local `spooled` receipt differs from a server `persisted` acknowledgement. Neither means human approval.

`audit_read_resource` accepts enrolled source identifiers rather than unrestricted URLs, preventing an audit endpoint from becoming an arbitrary network proxy. File paths, redirects, symlinks, upload sizes, decompression and content types require bounded validation. `audit_run_check` uses approved check definitions; it does not expose a privileged arbitrary shell. Check definitions themselves are versioned, and repository-provided checks execute as untrusted code in isolation.

## 5. Harness lifecycle

1. **Enroll and authenticate.** Establish developer/task authority, device identity, repository permissions, policy version and execution limits. A scheduled task identifies its service principal and sponsoring owner separately.
2. **Capture baseline.** Record repository/worktree identity, HEAD, index, dirty tracked files and permitted relevant untracked files. Capture instruction files and client configuration only where supported and allowed.
3. **Negotiate coverage.** Publish adapter version, available hooks, capture sources, exclusions and enforcement mode. Store an immutable initial capability manifest; changes create new events.
4. **Launch the client.** Pass scoped credentials and supported MCP configuration; record launch arguments after filtering. Propagate session and parent-run references to observed child runs.
5. **Observe and mediate.** Capture supported input/output and tool events; broker managed resource reads and checks; checkpoint workspace state. Record starts before dispatch and outcomes afterward where possible.
6. **Checkpoint changes.** Associate decisions and receipts with immutable snapshots. Observe edits from other processes when supported and otherwise expose attribution uncertainty.
7. **Prepare and bind.** Drain known pending operations, establish producer sequence boundaries, assemble a candidate, and verify the final commit tree before binding. Outstanding or unavailable producers remain declared gaps.
8. **Close and synchronize.** Record normal termination, cancellation or unexpected disconnect. Flush the encrypted spool and obtain server acknowledgements. Do not equate process exit with complete ingestion.

Session lifecycle: `created → active → closing → closed`, with interruption events and explicit resumed execution segments. Coverage and synchronization status are separate from lifecycle state. A closed session can still have missing evidence.

## 6. Execution receipts and snapshot binding

A harness receipt records operation ID, authenticated producer, parent operation, start/end observations, source/check definition version, input references, allowed parameters, environment identity, snapshot/tree digest, outcome, output artifact references, redaction policy, sequence and signature. Model-reported details are marked separately from harness-derived fields.

For checks, materialize an immutable workspace snapshot and execute in an isolated environment. If the check instead runs in a mutable workspace, record that limitation and concurrent changes; do not assert that its result validates the final commit. Record exit code separately from parsed assertions and label the parser/version. A zero exit code is not universally equivalent to passing tests.

Commit binding verifies repository identity, actual commit existence, parents and tree against the candidate snapshot using a trusted Git observation. Excluded inputs and external dependencies remain explicit. A mismatch produces a new snapshot and requires new applicable checks/review. A later squash or rebase requires a transformation record as defined in the product model.

The dossier manifest fixes event ranges, artifact digests, snapshot, decisions, capability manifests and policy version. Evidence arriving later creates a superseding dossier; it never silently changes an approved manifest.

## 7. Capturing what the developer saw

Store distinct events for `output_emitted`, `presentation_observed`, `review_requested`, and `human_disposition_recorded`. Client adapters may observe presentation only if the UI exposes a reliable signal. Terminal output or a transcript alone does not prove a person saw or understood the recommendation.

The review UI displays the proposal, concise rationale, cited evidence, affected snapshot and unresolved gaps. A separate authenticated human action records acceptance, amendment, rejection or deferral over that exact content. If the content changes while the review is open, reject stale submission and require refreshed review. The agent can poll the resulting status but cannot submit the human disposition.

## 8. Concrete example

For a transfer-limit change:

1. The harness records the task and baseline snapshot `S1` and starts agent run `A1`.
2. The agent calls `audit_read_resource` for an enrolled risk-policy version. The gateway returns filtered content and receipt `E1` showing the observed access.
3. The agent calls `audit_propose_decision`, citing `E1`, proposing a limit and documenting alternatives and risks. This creates proposal `D1`, not an accepted decision.
4. The agent edits code using the client's supported tools. The harness captures a patch and snapshot `S2`; if tool telemetry is missing, it reports that limitation.
5. `audit_run_check` runs a registered payments check against `S2`. Receipt `E2` records its environment and outcome.
6. The agent links `E2` to `D1` and requests review. The developer amends the limit in the review channel; subsequent code becomes `S3`, requiring applicable checks to run again.
7. The agent prepares a dossier for `S3` and binds it to a verified commit. The reviewer approves the final revision and dossier through the human channel.
8. CI independently verifies the revision, builds the artifact, and supplies the production provenance chain. Release gates inspect both policy outcome and evidence synchronization.

The resulting record distinguishes the agent's proposal, machine observations, the developer's amendment, the reviewer's approval and CI's build attestation.

## 9. Deployment and enforcement profiles

| Profile | Collection | Appropriate assurance claim |
| --- | --- | --- |
| Cooperative | Agent calls evidence tools; optional exports | Attributed statements and submitted artifacts; incomplete automatic visibility |
| Instrumented | Harness plus supported client hooks and managed operations | Observed activity within declared coverage, with bypasses still possible |
| Controlled | Isolated agent runtime, restricted egress/tools, protected harness and independent CI gates | Stronger enforcement over approved execution paths, subject to host and control-plane trust |

The selected profile is part of each dossier. Installing an MCP server must not silently label a repository as fully audited. For critical repositories, policies may require controlled execution plus independent CI evidence.

Unsupported clients can start in cooperative mode. Adapters for Claude Code, Codex and a specific Grok workflow require separate capability validation; no universal interception mechanism is assumed.

## 10. Failures, retries and adversarial behavior

- Same idempotency key and canonical payload returns the original result; a different payload with that key returns a conflict. Execution retries must not silently rerun checks with side effects.
- A lost response after dispatch yields an indeterminate operation until reconciled. Preserve the attempt record; do not fabricate a successful completion or claim exactly-once external execution.
- Network outages use an encrypted bounded spool. Required local persistence failure blocks managed operations under strict policy; server unavailability may permit local work under explicit offline policy but blocks release until required acknowledgements arrive.
- Collector crashes or missing sequence ranges create gaps. Heartbeats help detect interruption but cannot prove completeness. Capture-buffer exhaustion and filtering failures have explicit events and policy behavior.
- Agent-submitted “human approval” text is retained as a statement if permitted, never processed as an approval credential.
- Contradictory evidence is retained and surfaced. An agent's success summary cannot overwrite a failed check receipt.
- Child agents receive narrower scoped credentials and distinct identities. Unobserved delegation is a declared claim or gap, not a fabricated execution graph.
- Untrusted documentation and MCP responses cannot change capture policy, authorization, or retention settings. Summarizers receive read-only, restricted evidence access.

Only enrolled sources may produce privileged observations. The server validates signatures, key scope, producer sequences and replay boundaries. Canonical serialization and domain-separated signatures must be specified before implementation so independent verifiers agree on digests.

## 11. Rust implementation boundaries

| Proposed module | Responsibility |
| --- | --- |
| `audit-schema` | Versioned events, tool input/output types, receipts and canonical manifest format |
| `audit-harness` | Session lifecycle, sandbox integration, operation broker, snapshotting and credential isolation |
| `audit-collector` | Capture adapters, filtering, signed batches, encrypted spool and delivery acknowledgements |
| `audit-mcp` | MCP transport, tool/resource contracts, authorization and translation to domain services |
| `audit-adapters` | Client-specific capture and capability declarations |
| `audit-provenance` | Evidence graph, commit verification and dossier assembly |
| `audit-policy` | Deterministic evaluations over fixed manifests |

Keep shared domain services transport-independent so the MCP interface, web UI and CI integration enforce the same rules. Choose an MCP Rust library only after testing the required protocol, transport and authorization behavior; library selection is still open.

## 12. First deliverable and acceptance criteria

The first implementation slice would connect one validated client to the Rust harness and evidence MCP server, support context/decision submission, one managed resource read, one registered check, snapshot-bound commit dossiers, and a separate human review channel. Include encrypted buffering, provenance labels and visible gaps from the beginning. Full network mediation and multiple client adapters follow separately.

Before accepting that slice, demonstrate:

1. Agent statements cannot impersonate harness receipts or human approvals.
2. A resource read produces an access receipt; merely citing a URI does not.
3. A check receipt identifies the exact snapshot and cannot validate a different final tree.
4. Replayed requests do not duplicate evidence or silently rerun operations.
5. Cross-session and cross-tenant evidence references are rejected without disclosing restricted content.
6. A crash between operation start and finish yields an incomplete or indeterminate record.
7. Disabling capture, overflowing the spool or bypassing a managed tool exposes a gap to the extent the selected profile can detect it.
8. Agent-supplied logs retain their origin even when their contents resemble genuine harness output.
9. Sensitive content is filtered before persistent upload; raw capture handling follows declared local retention rules.
10. Human approval binds to the final dossier, and late evidence triggers the appropriate new evaluation and review.

Open design work: validate client hooks, select the first sandbox and operating system, define exact tool schemas and error codes, specify signature canonicalization, and set payload/retention/latency limits with pilot users. This document models those contracts without implementing them.
