# CLAUDE.md — miauDit

This repo builds miauDit, and it audits its own changes with it (dogfooding). Follow this on every task that modifies files here.

## The binary

Use the absolute path: **`~/.cargo/bin/miaudit`** (`~/.cargo/bin` is not on the user's PATH). The binary was formerly called `audit`, which collides with the macOS system tool `/usr/sbin/audit`; never run a bare `audit`. Reinstall after changing the CLI: `cargo install --path crates/miaudit-cli --locked` (and `crates/miaudit-ui` for the UI). On-disk data stays in `.audit/` and wire-format IDs keep the `auditAI/...` domain prefixes: both are format, not branding, and changing them would invalidate existing audit commits.

Always pass `--json` when you need to read results. Exit codes: 0 ok, 2 validation or integrity failure, 3 conflict, 4 partial verification.

## Audit workflow for each task that changes files

1. **Start:** `~/.cargo/bin/miaudit status --json`. If `NOT_INITIALIZED`, run `miaudit init` once.
2. **Context:** before or right after the first edit, record the request with `miaudit context add --file <json>`: what the user asked (their words, not an embellishment), constraints, and source references.
3. **Work**, then stage exactly what changed: `miaudit add <paths>`. Do not `miaudit add .` blindly; stage the files you actually modified or created. A deleted file is staged by passing its path.
4. **Decisions:** for each material choice (design, scope, trade-off, anything the user approved or rejected), `miaudit decision add --file <json>` with question, choice, alternatives, rationale, risks, and the **current** `snapshot_id` from `miaudit status`. If you stage more files afterwards the snapshot id changes and the commit fails with `SNAPSHOT_MISMATCH`; re-record the decisions against the new snapshot.
5. **Evidence:** attach real outputs with `miaudit evidence add` (test results, clippy output, command results). Copy actual output; never write evidence you did not observe. Link it to a decision with `evidence_links`/`decision_id`.
6. **Commit:** `miaudit commit -m "<what changed and why>"`, then `miaudit verify <audit_id>` and report the audit ID plus the integrity result to the user.
7. Report `miaudit gaps` honestly. Do not hide or minimize them.

## Linking audits to Git/GitHub commits

An audit commit says what was recorded about a change; a Git commit is where the change landed. Link them so anyone can tell which modifications belong to which commit.

- Only when the user asks you to `git commit` (never on your own): make the Git commit, then run `~/.cargo/bin/miaudit bind <audit_id> --git-commit HEAD`. Do this for every audit commit whose changes are in that Git commit.
- `bind` checks the Git commit's tree against the audit commit's changes (same content and mode; deletions must be absent) and records `full`, `partial` or `asserted`. Report which one you got. `partial` usually means the Git commit holds only some of the audited changes, or files changed after the audit.
- A `BINDING_MISMATCH` means that Git commit does not contain these changes. Do not "fix" it with `--assert`; ask the user, since `--assert` records an unverified claim.
- The GitHub link in a binding is derived from the `origin` remote. It does not prove the commit was pushed. Do not describe it as published unless you checked.
- To answer "which audits explain this commit": `miaudit related <git-rev>`. For the reverse: `miaudit bindings <audit_id>` or `miaudit show <audit_id>`.
- Bindings are separate attestations. They never change the audit manifest or its ID.

## Hook capture (observed events)

Optional, and **not active unless the user merges it into `.claude/settings.json`**: `crates/miaudit-cli/hooks/claude-code.settings.json` wires Claude Code hooks to `miaudit capture` (prompts, file edits, command digests) and `miaudit stop-check` (blocks the first Stop when edits were observed but not audited).

- Events are written by the harness through the hook, not by the agent, so they are labelled `observed`. They go to `.audit/events.jsonl`, a hash-chained append-only log. `miaudit events` shows chain status and recent events.
- No command text, tool output or file contents are stored, only digests; the user's prompt goes to an unencrypted blob. Redaction is still not implemented.
- `miaudit commit` attaches the new events as one `observed` evidence object and advances a cursor. Gaps cross-check events against the snapshot: `STAGED_WITHOUT_OBSERVED_EDIT`, `STAGED_DIFFERS_FROM_OBSERVED`, `OBSERVED_EDIT_NOT_STAGED`, `AUDIT_STORE_TOUCHED_BY_TOOL`, `EVENT_CHAIN_BROKEN`, `NO_OBSERVED_EVENTS`. Report them as they are.
- Limits: the agent runs as the same user and can rewrite `.audit/` (including recomputing the chain). `Bash` can change files that no edit event shows. The chain proves internal consistency, not completeness. Do not describe hook capture as tamper-proof.

## Rules that keep the audit honest

- Everything you submit is **self-reported** by you. Never present it as verified or approved. Do not write `approved_by`, `verified`, `origin` or similar fields; the schema rejects them on purpose.
- **Humans approve, not you.** There is no agent approval path. Never claim a change was approved or reviewed unless the user said so in this session, and then record that as their statement in a decision's rationale, attributed to them.
- Record failures and rejected alternatives too: a failed test, a reverted approach, something the user declined. Do not curate the history.
- Do not put secrets, tokens, credentials or personal data in context, decisions or evidence. **Redaction is not implemented**, and evidence content is stored unencrypted in `.audit/blobs`.
- The audit covers **only this repository** and **only staged paths**. Work done in other directories is out of scope; say so rather than implying coverage.
- An audit commit is not a Git commit and implies no policy success. Do not run `git commit` or push unless the user asks.
- `.audit/` is local evidence and is gitignored. Do not delete or edit files inside it; objects are immutable by design. If something was recorded wrongly, add a corrected record and say what superseded what.

## Project conventions

- Rust, edition 2024, unix only. Workspace: `crates/miaudit-schema` (types, canonical form, IDs), `crates/miaudit-cli` (`miaudit`), `crates/miaudit-ui` (read-only localhost viewer).
- Before finishing a task: `cargo test` and `cargo clippy --all-targets` must be clean, and the results should be attached as evidence.
- The design contract lives in `docs/`. When behaviour and docs disagree, surface it to the user instead of silently changing either.
- The UI is read-only and binds to `127.0.0.1` only. Keep it that way unless the user decides otherwise.
