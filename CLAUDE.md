# CLAUDE.md — miauDit

This repo builds miauDit, and it audits its own changes with it (dogfooding). Follow this on every task that modifies files here.

## The binary

Use the absolute path: **`~/.cargo/bin/miaudit`** (`~/.cargo/bin` is not on the user's PATH). The binary was formerly called `audit`, which collides with the macOS system tool `/usr/sbin/audit`; never run a bare `audit`. Reinstall after changing the CLI: `cargo install --path crates/miaudit-cli --locked` (and `crates/miaudit-ui` for the UI). Each repo keeps its data in a `.miaudit/` directory at its root, found by walking up from the current directory like `.git` (it was `.audit/` before; an old `.audit/` store is reported as `LEGACY_STORE`, fix it with `mv .audit .miaudit`). `miaudit init` writes `.miaudit/.gitignore` containing `*`, so the store never enters Git. Wire-format IDs keep the `auditAI/...` domain prefixes: that is format, not branding, and changing it would invalidate existing audit commits.

Always pass `--json` when you need to read results. Exit codes: 0 ok, 2 validation or integrity failure, 3 conflict, 4 partial verification.

## Project profile (ask the developer first)

When you start working on a project, check `miaudit project check`. `miaudit init` creates `.miaudit/PROJECT.md` as a template of questions, and the `SessionStart` hook (when active) reminds you if it is missing, incomplete or suspicious. If it is not `complete`, ask the developer before changing code, and write their answers into that file:

- how to run the server and the frontend locally, and how to confirm they are up;
- how to run the unit, integration and end-to-end tests;
- whether there is a staging environment, how to deploy to it, and how to tell which version runs there;
- which credentials are needed, as **names only** (environment variable, secret-store path, who provides it), never the values: the file is stored unencrypted and is not a place for secrets;
- how to collect evidence for a frontend change (screenshots: pages, login, viewport, who supplies the prints) and for an API change (real requests, before and after);
- how to run the previous version to get the "before" evidence;
- what must never be touched or run.

Use the developer's words; do not invent answers. If they decline or do not know, write that in the section (for example "no staging environment, confirmed by the developer") rather than leaving `TODO`, and do not mark something confirmed that they did not say. `miaudit project check` fails with exit 2 when it detects a secret-looking value (known token formats, private keys, labelled values) and reports only the line and kind; the check is a heuristic, not proof that the file is clean. Never paste a key the developer gives you into the file, into evidence or into a decision: tell them to export it as an environment variable and record the variable name. When running servers or collecting evidence, follow the profile; if it is incomplete, ask again instead of guessing. The profile contents are never injected into the context by the hook, only a fixed reminder.

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
- To link an audit to a GitHub pull request instead of a commit: `~/.cargo/bin/miaudit bind <audit_id> --pr <number|url>`. It asks `gh pr view` for the PR's head commit and runs the same check against that commit, so the head must be in the local repository (`git fetch origin pull/<n>/head` if not). The binding records the PR number, URL, head commit and state; it does not follow later pushes to the PR, and the state is as of binding time. `--git-commit` and `--pr` are mutually exclusive, and `bind-pending` only links commits.
- `bind` checks the Git commit's tree against the audit commit's changes (same content and mode; deletions must be absent) and records `full`, `partial` or `asserted`. Report which one you got. `partial` usually means the Git commit holds only some of the audited changes, or files changed after the audit.
- A `BINDING_MISMATCH` means that Git commit does not contain these changes. Do not "fix" it with `--assert`; ask the user, since `--assert` records an unverified claim.
- The GitHub link in a binding is derived from the `origin` remote. It does not prove the commit was pushed. Do not describe it as published unless you checked.
- `miaudit bind-pending [--git-commit HEAD]` links every still-unlinked audit commit whose changes match that Git commit fully, and only those; partial matches are listed as skipped and never recorded (use `bind` yourself if you want one). `miaudit git-hook install` writes `.git/hooks/post-commit` to run it after each `git commit`; it refuses to overwrite a different hook. It is not installed unless the user asks.
- To answer "which audits explain this commit": `miaudit related <git-rev>`. For the reverse: `miaudit bindings <audit_id>` or `miaudit show <audit_id>`.
- Bindings are separate attestations. They never change the audit manifest or its ID.

## GitHub user of each audit commit

`miaudit commit` records who made it in the manifest's `author` (part of the audit ID): the login and numeric id from `gh api user` (provenance `derived`, `gh_authenticated_session_unsigned`), or `MIAUDIT_GITHUB_USER` (provenance `self_reported`, no id), or `unavailable` with the reason when neither works. It never guesses from `git config`, and only the login and id are kept, no email. Gaps `AUTHOR_UNSIGNED`, `AUTHOR_ASSERTED` or `AUTHOR_UNAVAILABLE` say how much to trust it. This is not a signature: anyone who can run the CLI as that user, or set the variable, can claim the login. Audit commits made before this feature have no `author`.

## Hook capture (observed events)

Optional, and **not active unless the user merges it into `.claude/settings.json`**: `crates/miaudit-cli/hooks/claude-code.settings.json` wires Claude Code hooks to `miaudit capture` (prompts, file edits, command digests) and `miaudit stop-check` (blocks the first Stop when edits were observed but not audited).

- Codex: `crates/miaudit-cli/hooks/codex.hooks.json` is the equivalent for the OpenAI Codex CLI (copy to `.codex/hooks.json`, then trust it in `/hooks`). It runs `miaudit capture --agent codex`; `apply_patch` edits become one `file_edit` event per file in the patch (add, update, delete, move_from, move_to). Not installed, not exercised against a real Codex run.
- What the hooks record (all as digests, paths and hosts; no output, prompt-to-subagent or reply text, URL paths or queries): user prompts as blobs; commands as program, command digest, output digest and exit code when the agent reports it; file edits with the digest after; `Read`, `Grep`, `Glob`, `WebFetch`, `WebSearch` and MCP calls as `resource_access`; subagent spawns (`Task`, `Agent`, `spawn_agent`) and `SubagentStart`/`SubagentStop` as `agent` events; and at each `Stop` a digest of the last assistant message and of the transcript file. They back the coverage rows `conversation_capture`, `agent_topology`, `resource_access_receipts` and `command_execution_receipts`, shown as `observed` with a `partial (hook)` note and the gap `CAPTURE_PARTIAL:*`; without hook events they stay `unavailable`. A digest lets someone who holds the original text check it; it does not let anyone read it. Reads and network use inside shell commands are not seen.
- Events are written by the harness through the hook, not by the agent, so they are labelled `observed`. They go to `.miaudit/events.jsonl`, a hash-chained append-only log. `miaudit events` shows chain status and recent events.
- No command text, tool output or file contents are stored, only digests; the user's prompt goes to an unencrypted blob. Redaction is still not implemented.
- `miaudit commit` attaches the new events as one `observed` evidence object and advances a cursor. Gaps cross-check events against the snapshot: `STAGED_WITHOUT_OBSERVED_EDIT`, `STAGED_DIFFERS_FROM_OBSERVED`, `OBSERVED_EDIT_NOT_STAGED`, `AUDIT_STORE_TOUCHED_BY_TOOL`, `EVENT_CHAIN_BROKEN`, `NO_OBSERVED_EVENTS`. Report them as they are. `conversation_capture` and `command_execution_receipts` show as `observed` with a `partial:` note (gap `CAPTURE_PARTIAL:*`) when hook events of that kind exist, and `unavailable` otherwise. The `signatures` row was removed from the coverage table at the user's request; nothing is signed, and `verify` still reports `signatures: none`.
- Limits: the agent runs as the same user and can rewrite `.miaudit/` (including recomputing the chain). `Bash` can change files that no edit event shows. The chain proves internal consistency, not completeness. Do not describe hook capture as tamper-proof.

## Rules that keep the audit honest

- Everything you submit is **self-reported** by you. Never present it as verified or approved. Do not write `approved_by`, `verified`, `origin` or similar fields; the schema rejects them on purpose.
- **Humans approve, not you.** There is no agent approval path. Never claim a change was approved or reviewed unless the user said so in this session, and then record that as their statement in a decision's rationale, attributed to them.
- Record failures and rejected alternatives too: a failed test, a reverted approach, something the user declined. Do not curate the history.
- Do not put secrets, tokens, credentials or personal data in context, decisions or evidence. **Redaction is not implemented**, and evidence content is stored unencrypted in `.miaudit/blobs`.
- The audit covers **only this repository** and **only staged paths**. Work done in other directories is out of scope; say so rather than implying coverage.
- An audit commit is not a Git commit and implies no policy success. Do not run `git commit` or push unless the user asks.
- `.miaudit/` is local evidence and is gitignored. Do not delete or edit files inside it; objects are immutable by design. If something was recorded wrongly, add a corrected record and say what superseded what.

## Project conventions

- Rust, edition 2024, unix only. Workspace: `crates/miaudit-schema` (types, canonical form, IDs), `crates/miaudit-cli` (`miaudit`), `crates/miaudit-ui` (read-only localhost viewer).
- Before finishing a task, run the tests and attach what actually ran. Compiling is not testing: `cargo build`, `cargo check` and `cargo clippy` output proves the code compiles or passes static analysis, and must never be offered as test evidence (label clippy output as static analysis if you attach it at all). Test evidence is:
  - the full output of `cargo test --workspace`, with every test name and its result and the integration suites (`tests/*.rs`, which drive the real binary) visible, not only `test result:` summary lines;
  - the output of `scripts/e2e.sh`, which runs the installed `miaudit` and `miaudit-ui` end to end in a temporary Git repo (real hook payloads, commit, verify, tamper detection, Git hook binding, viewer over HTTP). Run `cargo install --path ...` first so it tests the current code.
  - In the evidence description say what was executed, how many tests passed and failed, and what was not run. If a test was skipped or no test covers the change, say so and record the gap; do not imply coverage.
  - Attach these outputs with `evidence add` using a `file`, copied from the real run.
- Evidence also depends on what kind of change it is. Declare it in the context as `change_kind` (`frontend`, `api` or `other`; leave `other` only when neither applies, for example docs or a CLI-only change). `miaudit gaps` then reports what is missing, and you must tell the user about each gap instead of hiding it:
  - **frontend:** a screenshot before and after the change, attached as an image file with `phase` (`before` or `after`) and `environment` (`local`, `staging` or `production`). Ask the developer for the prints, or capture them yourself with a browser or screenshot tool when one is available, and say which you did. Text, a mock-up or an image extension on non-image bytes does not count (the file content is checked). Gaps: `FRONTEND_EVIDENCE_MISSING:before|after`.
  - **api:** the real requests and responses of the old and the new version of the code run against the API, with `phase` and `environment` (`local`, `staging` or `production`; `ci` alone does not count). The `after` evidence must show the new version, so name the commit or snapshot that was run in the description. Gaps: `API_EVIDENCE_MISSING:before|after`.
  - **before means before:** capture the `before` evidence before you edit (the previous version running locally or on staging), not by reconstructing it afterwards. If you could not, record that as a gap and tell the user.
  - These fields are self-reported. The viewer shows the screenshots and the phase and environment, but nothing proves which version produced them.
  - `miaudit gaps` is advisory: `miaudit commit` still succeeds with these gaps. Do not commit an audit without this evidence unless the user explicitly waives it, and record the waiver as their statement.
- The design contract lives in `docs/`. When behaviour and docs disagree, surface it to the user instead of silently changing either.
- The UI is read-only and binds to `127.0.0.1` only. Keep it that way unless the user decides otherwise.
