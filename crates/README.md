# miauDit local core (first slice)

Offline implementation of the CLI core described in `docs/cli-and-normalized-api.md`. Rust, unix only.

- `miaudit-schema`: versioned types, strict closed-schema input validation, canonical serialization (`auditai-c14n-v1`), domain-separated SHA-256 IDs, tree digests and diffs.
- `miaudit-cli` (binary `miaudit`): `init`, `status`, `add`, `diff [--staged]`, `context|decision|evidence add --file`, `gaps`, `commit -m`, `log`, `show`, `verify`, Git linking (`bind`, `bindings`, `related`) and hook capture (`capture`, `stop-check`, `events`). Every command accepts `--json` (versioned envelope `contract_version: "1"`).
- `miaudit-ui` (binary `miaudit-ui`): read-only viewer on `http://127.0.0.1:7070`. See below.

Exit codes: `0` ok, `1` internal, `2` validation/integrity failure, `3` conflict, `4` insufficient evidence (partial verification).

## Linking audit commits to Git/GitHub commits
`miaudit bind <audit-id> --git-commit <rev>` compares the audit commit's changes with the Git commit's tree (same content and mode; deletions must be absent) and stores a **separate, immutable attestation** (`.audit/bindings/`), so the audit ID never changes. Result: `full`, `partial`, or `asserted` (only with `--assert`; a plain mismatch is rejected with `BINDING_MISMATCH`). Several audits may bind to one Git commit and vice versa.
`miaudit related <git-rev>` answers "which audits explain this commit". The GitHub URL is derived from `origin` (credentials are never stored) and does **not** prove the commit was pushed.

## Hook capture
`miaudit capture` is the entry point for Claude Code hooks: it reads the hook JSON on stdin and appends an `observed` event to `.audit/events.jsonl` (hash-chained, append-only, file-locked so parallel hooks are safe). It prints nothing to stdout (that would enter the model context) and exits 0 even on failure, which it records as a `capture_error` event. Only digests are stored for commands and file contents; the user prompt goes to an unencrypted blob. `miaudit stop-check` is the Stop hook: exit 2 with a stderr message once when edits were observed but not audited (it lets the agent stop when `stop_hook_active` is true). An example configuration is `miaudit-cli/hooks/claude-code.settings.json`; it is not installed anywhere automatically.
`miaudit commit` attaches the new events as one `observed` evidence object and advances `.audit/events.cursor`; `miaudit verify` re-checks the chain inside that evidence. Gaps compare events with the snapshot (see `CLAUDE.md`). The chain proves internal consistency only: the agent runs as the same user and could rewrite `.audit/` and recompute it, and `Bash` can change files that no edit event shows. Signing and a relay outside the agent's reach are not implemented.

## Viewer
`miaudit-ui --repo <path> [--port 7070]` serves a read-only page of audit commits, their Git links, context, decisions, evidence, gaps and a live `miaudit verify` result. GET only, `127.0.0.1` only, strict `Host` check against DNS rebinding, no JavaScript, all content HTML-escaped, strict CSP. It runs `~/.cargo/bin/miaudit` for verification (override with `--audit-bin`); a bare `miaudit` is never used because on macOS it resolves to the system tool.

## What it does not do (still open in the docs)
No MCP server, trusted harness, full execution receipts (only command digests via hooks), `check`, `push`/`fetch`, `review`/`approve`, policy evaluation, signatures or server. Local blobs are **not encrypted** and no retention policy is applied; both are reported as gaps in every audit commit.

## Trust model implemented here
- Everything supplied by a caller is stored as `self_reported`; only hook events written through `miaudit capture` are `observed`. File snapshots are labeled `unverified_client_snapshot`: the CLI is not a trusted harness.
- An audit commit never claims approval or policy success. `accepted`, `verified`, `policy_passed` and `human_approved` are separate results.
- The manifest excludes its own ID, signatures, Git bindings and approvals; those are separate attestations.
- Snapshot scope is **staged paths only**; `.git/` and `.audit/` are excluded and gitignore is not consulted. All of this is recorded in the manifest.
- Single writer: no inter-process locking.
