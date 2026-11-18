use crate::{error::Res, store::Store};
use std::{fs, path::PathBuf};

pub const PROFILE_FILE: &str = "PROJECT.md";

pub const SECTIONS: [(&str, &str); 9] = [
    ("Project", "What is this project, and which parts are frontend, API or something else? Which kind of change is typical (frontend, api, other)?"),
    ("Run locally", "How do you install dependencies and start the server and the frontend locally? Commands, URL and port, and how to confirm it is up."),
    ("Run tests", "Which commands run the unit, integration and end-to-end tests, and what must pass before a change is done?"),
    ("Staging", "Is there a staging environment? Its URL, how a branch is deployed to it, and how to tell which version is running."),
    ("Credentials", "NEVER write secret values here. For each key or token needed (staging API key, test user, database), give only the name of the environment variable, the secret-store path or who provides it."),
    ("Evidence: frontend", "How are screenshots taken (page URLs, login, viewport, tool)? Will the developer supply the prints, or may the agent capture them with a browser tool?"),
    ("Evidence: API", "How are real requests sent to the API before and after the change (curl examples, collection, base URL variable)? What does a good response look like?"),
    ("Before and after", "How is the previous version run to capture the 'before' evidence (git worktree, tag, deployed staging version, docker image)?"),
    ("Constraints", "What must never be touched or run (production, real customer data, paid services)?"),
];

pub fn template() -> String {
    let mut s = String::from("# Project profile for miauDit\n\nFilled in with the developer when work on this project starts. Never put secret values in this file: it is stored unencrypted. Replace every TODO.\n");
    for (h, q) in SECTIONS {
        s.push_str(&format!("\n## {h}\n> {q}\n\nTODO\n"));
    }
    s
}

pub fn path(store: &Store) -> PathBuf {
    store.dir.join(PROFILE_FILE)
}

pub fn init(store: &Store) -> Res<bool> {
    let p = path(store);
    if p.exists() {
        return Ok(false);
    }
    fs::write(&p, template())?;
    Ok(true)
}

pub struct Finding {
    pub line: usize,
    pub kind: &'static str,
}

pub struct Status {
    pub state: &'static str,
    pub incomplete: Vec<&'static str>,
    pub secrets: Vec<Finding>,
}

const KNOWN_PREFIXES: [&str; 14] = [
    "sk-", "sk_live_", "sk_test_", "rk_live_", "pk_live_", "ghp_", "gho_", "ghu_", "ghs_", "github_pat_", "xoxb-", "xoxp-", "AIza", "glpat-",
];

const LABELS: [&str; 9] = ["key", "token", "secret", "password", "passwd", "pwd", "apikey", "credential", "bearer"];

fn token_kind(t: &str) -> Option<&'static str> {
    if KNOWN_PREFIXES.iter().any(|p| t.starts_with(p)) && t.len() >= 16 {
        return Some("known_token_prefix");
    }
    if t.len() == 20 && t.starts_with("AKIA") && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
        return Some("aws_access_key");
    }
    if t.starts_with("eyJ") && t.len() >= 30 && t.matches('.').count() == 2 {
        return Some("jwt");
    }
    None
}

fn looks_like_secret_value(v: &str) -> bool {
    let v = v.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == ',' || c == ';');
    if v.len() < 16 || v.contains('/') || v.contains(' ') {
        return false;
    }
    if v.starts_with(['$', '<', '{', '[', '(']) {
        return false;
    }
    v.bytes().any(|b| b.is_ascii_digit()) && v.bytes().any(|b| b.is_ascii_uppercase()) && v.bytes().any(|b| b.is_ascii_lowercase())
}

pub fn scan_secrets(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
            out.push(Finding { line: n, kind: "private_key" });
            continue;
        }
        let mut found = None;
        for t in line.split(|c: char| !(c.is_ascii_alphanumeric() || "_-.+/".contains(c))) {
            if let Some(k) = token_kind(t) {
                found = Some(k);
                break;
            }
        }
        if found.is_none() {
            let lower = line.to_ascii_lowercase();
            for label in LABELS {
                let mut from = 0;
                while let Some(pos) = lower[from..].find(label) {
                    let after = from + pos + label.len();
                    let rest = line[after..].trim_start_matches(['"', '\'', '`', ' ']);
                    if let Some(val) = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) {
                        let val = val.trim_start();
                        let first = val.split_whitespace().next().unwrap_or("");
                        if looks_like_secret_value(first) {
                            found = Some("labelled_secret_value");
                        }
                    }
                    from = after;
                    if found.is_some() {
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
        }
        if let Some(k) = found {
            out.push(Finding { line: n, kind: k });
        }
    }
    out
}

pub fn incomplete_sections(text: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    for (h, _) in SECTIONS {
        let marker = format!("## {h}");
        let body: Option<Vec<&str>> = {
            let mut lines = text.lines();
            let mut found = false;
            let mut acc = Vec::new();
            for l in lines.by_ref() {
                if l.trim_end() == marker {
                    found = true;
                    break;
                }
            }
            if found {
                for l in lines {
                    if l.starts_with("## ") {
                        break;
                    }
                    acc.push(l);
                }
                Some(acc)
            } else {
                None
            }
        };
        let done = body.is_some_and(|b| {
            let answer: Vec<&&str> = b.iter().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('>')).collect();
            !answer.is_empty() && !answer.iter().any(|l| l.contains("TODO"))
        });
        if !done {
            missing.push(h);
        }
    }
    missing
}

pub fn status(store: &Store) -> Status {
    let Ok(text) = fs::read_to_string(path(store)) else {
        return Status { state: "missing", incomplete: SECTIONS.iter().map(|(h, _)| *h).collect(), secrets: vec![] };
    };
    let secrets = scan_secrets(&text);
    let incomplete = incomplete_sections(&text);
    let state = if !secrets.is_empty() {
        "contains_secret"
    } else if incomplete.is_empty() {
        "complete"
    } else {
        "incomplete"
    };
    Status { state, incomplete, secrets }
}

pub fn session_message(store: &Store) -> String {
    let rel = ".miaudit/PROJECT.md";
    let st = status(store);
    match st.state {
        "complete" => format!("miaudit: read {rel} before running the server or collecting evidence. It says how to run the project, collect evidence and reach staging. Do not write secret values into it."),
        "contains_secret" => format!("miaudit: {rel} looks like it contains a secret value. Tell the developer, replace it with the name of the environment variable, and run `miaudit project check`."),
        state => format!(
            "miaudit: the project profile {rel} is {state} (open sections: {}). Before changing code, ask the developer these questions, write the answers into {rel} and run `miaudit project check`: how to run the server and frontend locally, how to run the tests, whether there is a staging environment and how to deploy to it, which credentials are needed (names of environment variables or secret-store paths only, never the values), how to collect evidence for a frontend change (screenshots) and for an API change (real requests, before and after), and how to run the previous version for the 'before' evidence.",
            st.incomplete.join(", ")
        ),
    }
}
