# Prompts, responses, authorizations and review evidence

Status: design. Hook capture of prompts, replies, commands and reads is implemented in part (hash-only receipts; see [crates/README.md](../crates/README.md)); review-observation capture and approvals are not. Extends the [normalized API](cli-and-normalized-api.md) and [execution harness](evidence-mcp-and-harness.md). No implementation or UI is introduced.

## 1. Audit question

For a production change, reconstruct what the developer requested, what the AI returned, what the developer challenged or verified, what actions were authorized, what actually ran, and what files changed.

The record should let a reviewer distinguish an explicitly accepted initial suggestion from a suggestion revised after source inspection, testing or human feedback. It must also distinguish “no verification observed” from “verification did not occur.” Capture gaps and work outside the instrumented environment limit the conclusion.

Do not assign a developer diligence score or infer intent from prompt count, time spent, wording, or acceptance of the first answer. A first answer may already have independent verification; many follow-up prompts may add none. Preserve concrete actions and their outcomes so reviewers can make a contextual assessment.

## 2. Conversation records

Capture developer prompts and visible AI responses automatically through supported client adapters. Store permitted exact content as restricted artifacts, with searchable redacted derivatives where authorized. Preserve source message IDs, versions and ordering; summaries never replace the original available record.

Each message records:

- Conversation, session, agent run, message and parent-turn identifiers.
- Speaker role and authenticated or source-reported identity, clearly distinguished.
- Content artifact references, digest scheme, classification, redaction and availability.
- Source timestamps and collector receipt time.
- Related task, snapshot, decisions, source documents and operation IDs.
- Capture channel and whether content was emitted, delivered, presentation-observed, or unavailable.
- Completion state: complete, interrupted, truncated, or unknown.

Record developer corrections, edited prompts, regenerated responses and conversation branches as separate versions/events. Where supported, preserve stream fragments or a final assembled response plus explicit interruption metadata. Agent-visible tool results are separate messages linked to their execution receipts; a tool result is not a human statement.

Include project instructions, attachments, retrieved context and configuration only where exposed and permitted. Record context compaction and the summary supplied to the agent when observable; do not claim to know the model's complete input when the client withholds it. Private model reasoning is outside scope. Visible explanations and concise decision rationales are sufficient audit subjects.

The harness supplies authoritative capture labels. An agent importing a transcript through MCP creates an attributed import, not verified proof that those messages appeared in the client.

## 3. Observable review and hallucination-prevention evidence

Link review actions to the specific response, claim, decision and code revision they address:

| Observation | What it establishes | What it does not establish |
| --- | --- | --- |
| Developer requests sources or challenges an assumption | A recorded verification request | That the resulting sources are valid |
| Developer requests alternatives or flags a contradiction | A recorded challenge to the proposal | That the final choice is correct |
| Source is retrieved with a versioned access receipt | Observed access to a particular source | That it supports the AI's interpretation |
| Independent check runs on the affected snapshot | Recorded check execution and outcome | General correctness beyond the check |
| Developer amends or rejects a proposal | Explicit intervention over identified content | The intervention's correctness |
| Developer accepts the initial proposal | Explicit acceptance of that proposal | Absence of other review or verification |
| No review action appears in captured records | No such action was observed within declared coverage | Developer negligence or absence of off-platform review |

A review action includes actor, action kind, target references, supporting evidence, outcome, source message/span where applicable, and provenance. Examples of action kinds: `request_sources`, `challenge_assumption`, `request_alternative`, `request_check`, `inspect_diff`, `amend`, `reject`, and `explicit_acceptance`.

Natural-language classification is a derived annotation with extractor/version and source spans. Human corrections append superseding annotations. An AI classifier cannot turn “looks good” into a revision-bound approval; formal approval still uses the separately authenticated disposition endpoint.

Expose factual summaries such as “initial proposal accepted; two checks recorded against final snapshot; source validation unresolved.” Every summary statement links to events. Unresolved contradictions, failed checks and unavailable messages stay visible.

## 4. Command authorization is separate from execution

Model a command's lifecycle with distinct records:

1. **Proposed:** the agent or operator requests an action.
2. **Authorization requested:** the runtime presents a specific action and its scope to an eligible operator, if required.
3. **Disposition recorded:** the operator grants, denies or cancels; or a versioned standing policy supplies permission.
4. **Dispatched:** the harness attempts the authorized action after checking current permissions.
5. **Started:** an observer confirms process or remote-operation start.
6. **Completed, failed, cancelled or indeterminate:** preserve the actual result and any missing completion evidence.

A proposal or authorization does not prove execution. A command can run under a standing grant without a new prompt. A captured execution with no matching authorization is preserved and flagged for reconciliation or policy violation; it is not discarded to make the audit appear clean.

Authorization records contain requesting run, operator identity where applicable, exact displayed request, displayed scope, requested privileges, target environment, working directory, command digest, permission source, policy/grant version, decision time, expiry, usage limits and revocation events. Permission source distinguishes one-time human approval, standing human grant, organizational policy and unavailable authorization evidence.

One-time grants bind to immutable operation parameters. Prefix or category grants preserve their literal rule and evaluated scope; they must not be rendered as individual operator approval of every future command. The execution broker revalidates permissions immediately before dispatch and records which rule matched. Revocation affects future dispatch; already running operations receive a recorded cancellation attempt if supported.

Agent credentials cannot create operator grants. Human authorization provenance comes from a trusted runtime callback or the separate authenticated human channel. A conversation instruction may be retained as context but is not silently converted into an unlimited execution permission.

## 5. Actual command execution record

For every observed command retain:

- Operation/attempt ID, run and parent operation, authorization references and correlation confidence.
- Executable and argument vector, or exact submitted shell script artifact; shell identity/version and invocation mode.
- Working directory, repository/worktree, environment identity and requested sandbox/network privileges.
- Permitted environment metadata with secret values omitted or redacted; configuration references rather than credentials.
- Dispatch/start/end observations, process identity within its runtime, exit code or signal, timeout/cancellation and outcome provenance.
- stdout/stderr artifact references, truncation/redaction markers and digest scheme.
- Before/after snapshots and associated file-effect records.

Preserve pipelines, redirections and shell scripts as submitted. A parsed command list is a derived representation, not a substitute for the actual input. Shell parsing alone cannot prove which subprocesses executed; process observations, when available, supply separate child execution records.

Distinguish shell built-ins, local processes, remote jobs and MCP invocations. Capture MCP server/tool identity, version, normalized argument references, permission provenance and result receipts using the same operation linkage. A terminal's exit code is not automatically the outcome of every pipeline stage or remote side effect.

Broad arbitrary shell access weakens fine-grained authorization assurance. Controlled deployments enforce command/sandbox policies outside the agent; cooperative deployments disclose their inability to enforce or observe every subprocess.

## 6. Files and effects

Link actual file changes to command or editing-tool operations using the harness evidence. Each effect identifies path, create/modify/delete/type change, before/after digests, mode, snapshot IDs, observer and attribution basis.

Store permitted patches or restricted content artifacts for reconstruction. Hashes establish byte identity but cannot explain a change without accessible content or a diff. Record truncation, binary changes and content exclusions explicitly.

An isolated operation with stable before/after snapshots can support stronger attribution than a shared workspace diff. With concurrent agents or manual edits, preserve candidate operations and uncertainty; timing alone is insufficient to assign authorship. Include files modified and later reverted, even though they disappear from the final Git diff, when capture supports it.

Record observed effects outside the repository separately under policy. Network writes, database changes and remote actions require their own receipts; file snapshots do not capture those effects. Absence of an effect record is not proof that no effect occurred.

## 7. API and CLI additions

Extend the shared versioned schema with `ConversationMessage`, `MessageRevision`, `PresentationObservation`, `ReviewAction`, `AuthorizationRequest`, `AuthorizationDisposition`, `ExecutionAttempt`, and `FileEffect`. All records use scoped immutable references and append-only corrections.

Proposed API operations:

| Endpoint | Allowed authority |
| --- | --- |
| `POST /v1/conversation-events` | Enrolled adapters for observed messages; agent imports labeled separately |
| `POST /v1/review-actions` | Attributed human actions or explicitly derived classifications |
| `POST /v1/authorization-requests` | Scoped execution broker/requester; grants no permission by itself |
| `POST /v1/authorization-dispositions` | Authenticated eligible human or trusted policy evaluator |
| `POST /v1/execution-events` | Enrolled execution observers; agent claims use ordinary evidence submission |
| `POST /v1/file-effects` | Enrolled observers or explicitly unverified submissions |
| `GET /v1/audit-commits/{id}/interactions` | Authorized read of linked conversation and review events |
| `GET /v1/audit-commits/{id}/operations` | Authorized read of permissions, executions and effects |

Enforce producer restrictions per record variant; do not trust an input `role`, `origin`, `approved_by` or `verified` field. Reuse idempotency, strict schemas, reference checks and snapshot preconditions from the normalized API contract.

Proposed CLI reads: `audit conversation <audit-id>`, `audit commands <audit-id>`, `audit permissions <audit-id>`, `audit files <audit-id>`, and `miaudit review-evidence <audit-id>`. Support text and versioned JSON output, restricted artifact access and explicit coverage indicators. No UI is required.

Each audit manifest references a fixed set of conversation, authorization, execution and effect events. Later discoveries produce a superseding dossier or additional attestation, never a silent rewrite of reviewed history.

## 8. Example reconstruction

A developer asks an agent to update a payments SDK. The initial response proposes an API that does not exist. The developer asks for the official versioned documentation. A managed read records the source; the agent corrects its proposal.

The agent then requests a test command. The operator authorizes that exact command in the repository sandbox. The harness records dispatch, process start, failed tests, output and file snapshots. A second edit and check pass on the final snapshot. The developer explicitly accepts the corrected decision and a reviewer approves the bound revision.

The dossier preserves the initial incorrect response, the developer's challenge, source access, correction, authorization, both test attempts and all observed file effects. It does not omit the failure or attribute the correction solely to the AI.

If the developer instead accepts the first response, the dossier records that action and any available checks. It reports missing verification evidence where applicable, without inferring the developer's motivation or unobserved activity.

## 9. Privacy and acceptance criteria

Prompts and outputs can expose customer information, secrets and personal data. Apply the existing local filtering, access control, retention, legal-hold and artifact-deletion rules. Record whether exact content, redacted content or metadata alone is available. Restrict conversation export and audit access to it.

Before accepting this capture model, demonstrate that:

1. Edited/regenerated messages preserve lineage and do not overwrite prior available versions.
2. An agent-authored approval statement cannot become an operator grant.
3. Authorized-but-unexecuted commands and executed commands are distinguishable.
4. Standing grants and one-time permissions show their actual scope and matched policy.
5. Failed, cancelled and indeterminate execution attempts remain visible.
6. Commands, checks and file effects bind to the correct snapshots, including reverted intermediate changes where observed.
7. Concurrent edits and unsupported capture produce uncertainty rather than invented attribution.
8. Review summaries link to original evidence and disclose classification and capture limitations.
9. Acceptance of an initial response does not automatically produce a negligence label or a claim that verification was absent.
10. Redaction and retention do not leave unrestricted prompt copies in derived summaries, exports or search indexes.
