# miauDit local core (first slice)

Offline implementation of the CLI core described in `docs/cli-and-normalized-api.md`. Rust, unix only.

- `miaudit-schema`: versioned types, strict closed-schema input validation, canonical serialization (`auditai-c14n-v1`), domain-separated SHA-256 IDs, tree digests and diffs.
- `miaudit-cli` (binary `miaudit`): `init`, `status`, `add`, `diff [--staged]`, `context|decision|evidence add --file`, `gaps`, `commit -m`, `log`, `show`, `verify`, Git linking (`bind`, `bind-pending`, `git-hook install`, `bindings`, `related`) and hook capture (`capture`, `stop-check`, `events`). Every command accepts `--json` (versioned envelope `contract_version: "1"`).
- `miaudit-ui` (binary `miaudit-ui`): read-only viewer on `http://127.0.0.1:7070`. See below.

Exit codes: `0` ok, `1` internal, `2` validation/integrity failure, `3` conflict, `4` insufficient evidence (partial verification).

## Linking audit commits to Git/GitHub commits
`miaudit bind <audit-id> --git-commit <rev>` compares the audit commit's changes with the Git commit's tree (same content and mode; deletions must be absent) and stores a **separate, immutable attestation** (`.miaudit/bindings/`), so the audit ID never changes. Result: `full`, `partial`, or `asserted` (only with `--assert`; a plain mismatch is rejected with `BINDING_MISMATCH`). Several audits may bind to one Git commit and vice versa.
`miaudit related <git-rev>` answers "which audits explain this commit". The GitHub URL is derived from `origin` (credentials are never stored) and does **not** prove the commit was pushed.

## Tests
`cargo test --workspace` runs unit tests and integration suites that drive the real `miaudit` and `miaudit-ui` binaries (`tests/*.rs`). `scripts/e2e.sh` is an end-to-end run of the installed binaries in a temporary Git repo: hook payloads, audit commit, verify, tamper detection, post-commit binding and the viewer over HTTP. Building or linting is not evidence that tests ran.

## Project profile
`miaudit init` (or `miaudit project init`) creates `.miaudit/PROJECT.md`, a template of questions for the developer: run locally, run tests, staging, credentials (names only), how to collect frontend and API evidence, how to run the previous version, constraints. `miaudit project check` reports `missing`, `incomplete`, `complete` or `contains_secret` (exit 0, 4, 4, 2) and adds the gaps `PROJECT_PROFILE_MISSING`, `PROJECT_PROFILE_INCOMPLETE` and `PROJECT_PROFILE_SECRET` to `status` and to every audit commit. Secret detection is a heuristic (known token prefixes, AWS keys, JWTs, private keys, labelled values) and never echoes the value. `miaudit session-start` is the `SessionStart` hook: it prints a fixed reminder (the questions when the profile is not complete, otherwise a pointer to read it) and never the file content. Editing `.miaudit/PROJECT.md` through a tool is not flagged as tampering with the store.

## Evidence by kind of change
A context can declare `change_kind` (`frontend`, `api`, `other`) and evidence can declare `phase` (`before`, `after`) and `environment` (`local`, `staging`, `production`, `ci`). For `frontend`, `miaudit gaps` expects an image (checked by content, not extension) before and after; for `api`, the output of the old and new version run against the API, in `local`, `staging` or `production`. Missing pieces appear as `FRONTEND_EVIDENCE_MISSING:*` and `API_EVIDENCE_MISSING:*`; an undeclared kind is `CHANGE_KIND_UNDECLARED`. The viewer renders image evidence through a GET-only route that serves only blobs referenced by that audit whose digest matches and whose bytes are a PNG, JPEG, GIF or WebP (never SVG). Self-reported metadata: nothing proves which version produced a screenshot.

## Store and author
Each repository has a `.miaudit/` directory at its root (found by walking up, like `.git`) that ignores itself through `.miaudit/.gitignore`. Each audit commit records the GitHub user in `author`: `gh api user` login and id (`derived`), `MIAUDIT_GITHUB_USER` (`self_reported`), or `unavailable`. It is not signed.

## Hook capture
`miaudit capture` is the entry point for Claude Code and Codex hooks. Besides prompts, edits and commands (now with output digest and exit code), it records `Read`/`Grep`/`Glob`/`WebFetch`/`WebSearch`/MCP calls (paths, hosts, digests), subagent spawns and `SubagentStart`/`SubagentStop`, and `stop-check` records a digest of the last assistant message and of the transcript at each `Stop`. Everything is a hash, like Git objects: no output text, URL paths or reply text is stored. It reads the hook JSON on stdin and appends an `observed` event to `.miaudit/events.jsonl` (hash-chained, append-only, file-locked so parallel hooks are safe). It prints nothing to stdout (that would enter the model context) and exits 0 even on failure, which it records as a `capture_error` event. Only digests are stored for commands and file contents; the user prompt goes to an unencrypted blob. `miaudit stop-check` is the Stop hook: exit 2 with a stderr message once when edits were observed but not audited (it lets the agent stop when `stop_hook_active` is true). Example configurations: `miaudit-cli/hooks/claude-code.settings.json` and `miaudit-cli/hooks/codex.hooks.json`; nothing is installed automatically. `capture --agent codex` understands the Codex payload (`apply_patch` patch text is parsed for file paths; only paths and digests are stored, never the patch body). Codex needs the hook trusted in `/hooks` and the Codex payload shape was taken from its public documentation, not from a live run.
`miaudit commit` attaches the new events as one `observed` evidence object and advances `.miaudit/events.cursor`; `miaudit verify` re-checks the chain inside that evidence. Gaps compare events with the snapshot (see `CLAUDE.md`). The chain proves internal consistency only: the agent runs as the same user and could rewrite `.miaudit/` and recompute it, and `Bash` can change files that no edit event shows. Signing and a relay outside the agent's reach are not implemented.

## Viewer
`miaudit-ui --repo <path> [--port 7070]` serves a read-only page of audit commits, their Git links, context, decisions, evidence, gaps and a live `miaudit verify` result. GET only, `127.0.0.1` only, strict `Host` check against DNS rebinding, no JavaScript, all content HTML-escaped, strict CSP. It runs `~/.cargo/bin/miaudit` for verification (override with `--audit-bin`); a bare `miaudit` is never used because on macOS it resolves to the system tool.

## What it does not do (still open in the docs)
No MCP server, trusted harness, full execution receipts (only command digests via hooks), `check`, `push`/`fetch`, `review`/`approve`, policy evaluation, signatures or server. Local blobs are **not encrypted** and no retention policy is applied; the capture coverage table and gaps no longer list `local_encryption` or `developer_review_observation`, so this README and `CLAUDE.md` are the only places that say blobs are unencrypted and that no review or approval flow exists.

## Trust model implemented here
- Everything supplied by a caller is stored as `self_reported`; only hook events written through `miaudit capture` are `observed`. File snapshots are labeled `unverified_client_snapshot`: the CLI is not a trusted harness.
- An audit commit never claims approval or policy success. `accepted`, `verified`, `policy_passed` and `human_approved` are separate results.
- The manifest excludes its own ID, signatures, Git bindings and approvals; those are separate attestations.
- Snapshot scope is **staged paths only**; `.git/` and `.miaudit/` are excluded and gitignore is not consulted. All of this is recorded in the manifest.
- Single writer: no inter-process locking.
