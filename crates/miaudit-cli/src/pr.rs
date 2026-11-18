use crate::error::{CliError, Res};
use miaudit_schema::model::PullRequestRef;
use serde_json::Value;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const GH_TIMEOUT: Duration = Duration::from_secs(15);

fn valid_arg(arg: &str) -> bool {
    let number = !arg.is_empty() && arg.len() <= 9 && arg.bytes().all(|b| b.is_ascii_digit());
    let url = arg.starts_with("https://github.com/") && arg.contains("/pull/") && arg.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'));
    number || url
}

fn valid_pr_url(url: &str) -> bool {
    url.starts_with("https://github.com/") && url.contains("/pull/") && url.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'))
}

fn unresolvable(why: impl Into<String>) -> CliError {
    CliError::validation("PR_UNRESOLVABLE", why)
}

pub fn resolve(root: &Path, arg: &str) -> Res<PullRequestRef> {
    if !valid_arg(arg) {
        return Err(CliError::validation("BAD_PR", "pass a pull request number or a https://github.com/<owner>/<repo>/pull/<n> URL").field("/pr"));
    }
    let mut child = Command::new("gh")
        .args(["pr", "view", arg, "--json", "number,url,headRefOid,state"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| unresolvable(format!("gh could not be started: {e}")))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > GH_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unresolvable("gh pr view timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(unresolvable(format!("gh wait failed: {e}"))),
        }
    };
    if !status.success() {
        return Err(unresolvable(format!("gh pr view {arg} failed (not logged in, offline, or no such pull request)")));
    }
    let mut out = Vec::new();
    child.stdout.take().ok_or_else(|| unresolvable("no gh output"))?.take(1024 * 1024).read_to_end(&mut out).map_err(|e| unresolvable(e.to_string()))?;
    let v: Value = serde_json::from_slice(&out).map_err(|_| unresolvable("gh pr view returned invalid JSON"))?;
    let number = v["number"].as_u64().ok_or_else(|| unresolvable("gh returned no pull request number"))?;
    let web_url = v["url"].as_str().filter(|u| valid_pr_url(u)).ok_or_else(|| unresolvable("gh returned no valid github.com pull request URL"))?.to_string();
    let head = v["headRefOid"].as_str().filter(|h| matches!(h.len(), 40 | 64) && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).ok_or_else(|| unresolvable("gh returned no valid head commit"))?.to_string();
    let state = v["state"].as_str().filter(|s| s.len() <= 20 && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')).unwrap_or("UNKNOWN").to_string();
    Ok(PullRequestRef { number, web_url, head_commit: head, state })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_numbers_and_github_pull_urls_only() {
        for ok in ["7", "123456789", "https://github.com/acme/repo/pull/7"] {
            assert!(valid_arg(ok), "{ok}");
        }
        for bad in ["", "-7", "7; rm", "1234567890", "--web", "https://evil.com/acme/repo/pull/7", "https://github.com/acme/repo/issues/7", "https://github.com/a b/pull/7"] {
            assert!(!valid_arg(bad), "{bad}");
        }
    }
}
