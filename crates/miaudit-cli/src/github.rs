use miaudit_schema::model::{Author, Provenance};
use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const GH_TIMEOUT: Duration = Duration::from_secs(5);

pub fn valid_login(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn unavailable(reason: impl Into<String>) -> Author {
    Author {
        github_login: None,
        github_id: None,
        source: "unavailable".into(),
        provenance: Provenance::Unavailable,
        verification: reason.into(),
    }
}

fn gh_user() -> Result<(String, u64), String> {
    let mut child = Command::new("gh")
        .args(["api", "user"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("gh could not be started: {e}"))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > GH_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gh api user timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("gh wait failed: {e}")),
        }
    };
    if !status.success() {
        return Err("gh api user failed (not logged in, or offline)".into());
    }
    let mut out = Vec::new();
    child.stdout.take().ok_or("no gh output")?.take(1024 * 1024).read_to_end(&mut out).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&out).map_err(|_| "gh api user returned invalid JSON".to_string())?;
    let login = v.get("login").and_then(Value::as_str).filter(|l| valid_login(l)).ok_or("gh api user returned no valid login")?;
    let id = v.get("id").and_then(Value::as_u64).ok_or("gh api user returned no id")?;
    Ok((login.to_string(), id))
}

pub fn resolve_author_without_network() -> Author {
    resolve(false)
}

pub fn resolve_author() -> Author {
    resolve(true)
}

fn resolve(ask_gh: bool) -> Author {
    if let Ok(v) = std::env::var("MIAUDIT_GITHUB_USER") {
        let login = v.trim();
        return if valid_login(login) {
            Author {
                github_login: Some(login.into()),
                github_id: None,
                source: "env:MIAUDIT_GITHUB_USER".into(),
                provenance: Provenance::SelfReported,
                verification: "asserted_by_environment_unverified".into(),
            }
        } else {
            unavailable("MIAUDIT_GITHUB_USER is not a valid GitHub login")
        };
    }
    if !ask_gh {
        return unavailable("checkpoint: the gh lookup is skipped so the Stop hook stays fast");
    }
    match gh_user() {
        Ok((login, id)) => Author {
            github_login: Some(login),
            github_id: Some(id),
            source: "gh_api_user".into(),
            provenance: Provenance::Derived,
            verification: "gh_authenticated_session_unsigned".into(),
        },
        Err(reason) => unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::valid_login;

    #[test]
    fn login_validation() {
        for ok in ["a", "octo-cat", "A1", "x".repeat(39).as_str()] {
            assert!(valid_login(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", "a b", "a/b", "a_b", "ação", "x".repeat(40).as_str(), "a\n"] {
            assert!(!valid_login(bad), "{bad:?}");
        }
    }
}
