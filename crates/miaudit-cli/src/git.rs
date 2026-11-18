
use crate::error::{CliError, Res};
use miaudit_schema::{digest::content_id, model::{Change, Operation, PathState}};
use std::{path::Path, process::{Command, Output}};

fn run(root: &Path, args: &[&str]) -> Res<Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| CliError::validation("GIT_UNAVAILABLE", "git executable not available"))
}

pub fn resolve_commit(root: &Path, rev: &str) -> Res<(String, String)> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains('\0') || rev.chars().any(char::is_whitespace) {
        return Err(CliError::validation("GIT_COMMIT_NOT_FOUND", "invalid git revision").field("/git_commit"));
    }
    let spec = format!("{rev}^{{commit}}");
    let out = run(root, &["rev-parse", "--verify", "--quiet", &spec])?;
    let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || commit.is_empty() {
        return Err(CliError::validation("GIT_COMMIT_NOT_FOUND", format!("{rev}: not a commit in this repository")).field("/git_commit"));
    }
    let algorithm = if commit.len() == 64 { "sha256" } else { "sha1" };
    Ok((commit, algorithm.into()))
}

pub fn github_web_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    let rest = if let Some(r) = s.strip_prefix("git@github.com:") {
        r
    } else if let Some(r) = s.strip_prefix("ssh://git@github.com/") {
        r
    } else if let Some(r) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
        let (host, path) = r.split_once('/')?;
        if host.rsplit('@').next()? != "github.com" {
            return None;
        }
        path
    } else {
        return None;
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    (ok(owner) && ok(repo)).then(|| format!("https://github.com/{owner}/{repo}"))
}

pub fn repository_web_url(root: &Path) -> Option<String> {
    let out = run(root, &["remote", "get-url", "origin"]).ok()?;
    out.status.success().then(|| github_web_url(&String::from_utf8_lossy(&out.stdout))).flatten()
}

fn tree_entry(root: &Path, commit: &str, path: &str) -> Res<Option<(String, String, String)>> {
    let out = run(root, &["ls-tree", "-z", commit, "--", path])?;
    if !out.status.success() {
        return Err(CliError::internal("git ls-tree failed"));
    }
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let text = String::from_utf8_lossy(rec);
        let Some((meta, name)) = text.split_once('\t') else { continue };
        if name != path {
            continue;
        }
        let mut it = meta.split(' ');
        if let (Some(mode), Some(kind), Some(oid)) = (it.next(), it.next(), it.next()) {
            return Ok(Some((mode.into(), kind.into(), oid.into())));
        }
    }
    Ok(None)
}

pub fn check(root: &Path, commit: &str, ch: &Change) -> Res<PathState> {
    let entry = tree_entry(root, commit, &ch.path)?;
    if ch.operation == Operation::Delete {
        return Ok(if entry.is_none() { PathState::Match } else { PathState::PresentInGit });
    }
    let Some((mode, kind, oid)) = entry else { return Ok(PathState::AbsentInGit) };
    if kind != "blob" {
        return Ok(PathState::ContentDiffers);
    }
    let out = run(root, &["cat-file", "blob", &oid])?;
    if !out.status.success() {
        return Err(CliError::internal("git cat-file failed"));
    }
    if Some(content_id(&out.stdout)) != ch.digest_after {
        return Ok(PathState::ContentDiffers);
    }
    if Some(mode) != ch.mode_after.map(|m| format!("{m:o}")) {
        return Ok(PathState::ModeDiffers);
    }
    Ok(PathState::Match)
}

#[cfg(test)]
mod tests {
    use super::github_web_url as w;

    #[test]
    fn parses_github_remotes_and_drops_credentials() {
        let want = Some("https://github.com/acme/repo".to_string());
        assert_eq!(w("git@github.com:acme/repo.git"), want);
        assert_eq!(w("ssh://git@github.com/acme/repo"), want);
        assert_eq!(w("https://github.com/acme/repo.git\n"), want);
        assert_eq!(w("https://user:s3cret@github.com/acme/repo.git"), want);
        assert_eq!(w("https://gitlab.com/acme/repo.git"), None);
        assert_eq!(w("https://evil.com/github.com/acme/repo"), None);
        assert_eq!(w("https://github.com/acme/repo/extra"), None);
        assert_eq!(w("/local/path/repo"), None);
    }
}
