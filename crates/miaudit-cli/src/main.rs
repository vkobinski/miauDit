mod commands;
mod error;
mod events;
mod git;
mod github;
mod project;
mod push;
mod remote;
mod snapshot;
mod store;

use clap::{Parser, Subcommand};
use commands::{Ctx, Out};
use serde_json::{Value, json};

const CONTRACT_VERSION: &str = miaudit_schema::CONTRACT_VERSION;

#[derive(Parser)]
#[command(name = "miaudit", about = "miauDit local audit core (provenance for AI-assisted changes)")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init,
    Status,
    Add { paths: Vec<String> },
    Diff {
        #[arg(long)]
        staged: bool,
    },
    Context { #[command(subcommand)] op: AddOp },
    Decision { #[command(subcommand)] op: AddOp },
    Evidence { #[command(subcommand)] op: AddOp },
    Gaps,
    Commit {
        #[arg(short = 'm', long)]
        message: String,
    },
    Log,
    Bind {
        audit_id: String,
        #[arg(long = "git-commit")]
        git_commit: String,
        #[arg(long)]
        assert: bool,
    },
    Bindings { audit_id: Option<String> },
    BindPending {
        #[arg(long = "git-commit", default_value = "HEAD")]
        git_commit: String,
    },
    GitHook { #[command(subcommand)] op: GitHookOp },
    Related { git_commit: String },
    Events,
    Capture {
        #[arg(long, value_enum, default_value = "claude-code")]
        agent: events::Agent,
    },
    StopCheck {
        #[arg(long, value_enum, default_value = "claude-code")]
        agent: events::Agent,
    },
    SessionStart,
    Project { #[command(subcommand)] op: ProjectOp },
    Show { id: String },
    Verify { id: String },
    Push {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    Remote {
        #[arg(long, global = true)]
        server: Option<String>,
        #[command(subcommand)]
        op: RemoteOp,
    },
}

#[derive(Subcommand)]
enum RemoteOp {
    Repos,
    Audits {
        repo: String,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        gap: Option<String>,
        #[arg(long, default_value_t = 1)]
        page: u64,
        #[arg(long)]
        all: bool,
    },
    Show {
        repo: String,
        audit_id: String,
    },
}

#[derive(Subcommand)]
enum ProjectOp {
    Init,
    Check,
}

#[derive(Subcommand)]
enum GitHookOp {
    Install,
}

#[derive(Subcommand)]
enum AddOp {
    Add {
        #[arg(long)]
        file: String,
    },
}

fn run(cli: &Cli) -> error::Res<Out> {
    let cwd = std::env::current_dir()?;
    if let Cmd::Init = cli.cmd {
        return commands::init(&cwd);
    }
    if let Cmd::Remote { server, op } = &cli.cmd {
        let r = remote::Remote::new(server.as_deref())?;
        return match op {
            RemoteOp::Repos => remote::repos(&r),
            RemoteOp::Audits { repo, author, gap, page, all } => remote::audits(&r, &remote::AuditsQuery { slug: repo, author: author.as_deref(), gap: gap.as_deref(), page: *page, all: *all }),
            RemoteOp::Show { repo, audit_id } => remote::show(&r, repo, audit_id),
        };
    }
    let ctx = Ctx::open(&cwd)?;
    match &cli.cmd {
        Cmd::Init => unreachable!(),
        Cmd::Status => commands::status(&ctx),
        Cmd::Add { paths } => commands::add(&ctx, paths),
        Cmd::Diff { staged } => commands::diff(&ctx, *staged),
        Cmd::Context { op: AddOp::Add { file } } => commands::context_add(&ctx, file),
        Cmd::Decision { op: AddOp::Add { file } } => commands::decision_add(&ctx, file),
        Cmd::Evidence { op: AddOp::Add { file } } => commands::evidence_add(&ctx, file),
        Cmd::Gaps => commands::gaps(&ctx),
        Cmd::Commit { message } => commands::commit(&ctx, message),
        Cmd::Log => commands::log(&ctx),
        Cmd::Bind { audit_id, git_commit, assert } => commands::bind(&ctx, audit_id, git_commit, *assert),
        Cmd::BindPending { git_commit } => commands::bind_pending(&ctx, git_commit),
        Cmd::GitHook { op: GitHookOp::Install } => commands::git_hook_install(&ctx),
        Cmd::Bindings { audit_id } => commands::bindings(&ctx, audit_id.as_deref()),
        Cmd::Related { git_commit } => commands::related(&ctx, git_commit),
        Cmd::Events => commands::events_status(&ctx),
        Cmd::Project { op: ProjectOp::Init } => commands::project_init(&ctx),
        Cmd::Project { op: ProjectOp::Check } => commands::project_check(&ctx),
        Cmd::Capture { .. } | Cmd::StopCheck { .. } | Cmd::SessionStart => unreachable!("handled before the JSON envelope"),
        Cmd::Show { id } => commands::show(&ctx, id),
        Cmd::Verify { id } => commands::verify(&ctx, id),
        Cmd::Push { server, name } => push::push(&ctx, server.as_deref(), name.as_deref()),
        Cmd::Remote { .. } => unreachable!(),
    }
}

fn render(v: &Value, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                match v {
                    Value::Object(_) | Value::Array(_) if !v.as_array().is_some_and(|a| a.is_empty()) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        render(v, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}{k}: {}\n", scalar(v))),
                }
            }
        }
        Value::Array(a) => {
            for item in a {
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        out.push_str(&format!("{pad}-\n"));
                        render(item, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}- {}\n", scalar(item))),
                }
            }
        }
        _ => out.push_str(&format!("{pad}{}\n", scalar(v))),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) if a.is_empty() => "[]".into(),
        other => other.to_string(),
    }
}

fn run_hook(cmd: &Cmd) -> ! {
    use std::io::Read;
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut buf = Vec::new();
    let read = if std::io::IsTerminal::is_terminal(&std::io::stdin()) { Ok(0) } else { std::io::stdin().take(events::MAX_HOOK_INPUT + 1).read_to_end(&mut buf) };
    let input: Option<Value> = if read.is_ok() && buf.len() as u64 <= events::MAX_HOOK_INPUT { serde_json::from_slice(&buf).ok() } else { None };
    let hook_cwd = input.as_ref().and_then(|v| v.get("cwd")).and_then(Value::as_str).map(std::path::PathBuf::from).unwrap_or(cwd);
    let Ok(ctx) = Ctx::open(&hook_cwd) else {
        eprintln!("miaudit: no .miaudit store found from {}; hook ignored", hook_cwd.display());
        std::process::exit(0);
    };
    match cmd {
        Cmd::Capture { agent } => {
            let res = match &input {
                Some(v) => events::capture(&ctx.store, &ctx.cwd, v, *agent).map(|_| ()),
                None => events::capture_unparsed(&ctx.store, "hook input was not valid JSON or exceeded the size limit", buf.len() as u64, *agent).map(|_| ()),
            };
            if let Err(e) = res {
                eprintln!("miaudit: capture failed: {}", e.rule);
            }
            std::process::exit(0)
        }
        Cmd::SessionStart => {
            print!("{}", project::session_message(&ctx.store));
            std::process::exit(0)
        }
        Cmd::StopCheck { agent } => {
            if let Some(v) = &input
                && let Err(e) = events::capture_stop(&ctx.store, v, *agent)
            {
                eprintln!("miaudit: stop capture failed: {}", e.rule);
            }
            if input.as_ref().and_then(|v| v.get("stop_hook_active")).and_then(Value::as_bool) == Some(true) {
                std::process::exit(0);
            }
            match commands::stop_check(&ctx) {
                Some(msg) => {
                    eprintln!("{msg}");
                    std::process::exit(2)
                }
                None => std::process::exit(0),
            }
        }
        _ => std::process::exit(0),
    }
}

fn main() {
    let cli = Cli::parse();
    if matches!(cli.cmd, Cmd::Capture { .. } | Cmd::StopCheck { .. } | Cmd::SessionStart) {
        run_hook(&cli.cmd);
    }
    let (envelope, exit) = match run(&cli) {
        Ok(Out { value, exit }) => (json!({"contract_version": CONTRACT_VERSION, "ok": exit == 0, "result": value}), exit),
        Err(e) => (json!({"contract_version": CONTRACT_VERSION, "ok": false, "error": e.to_json()}), e.exit),
    };
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
    } else if let Some(lines) = envelope["result"]["lines"].as_array() {
        for l in lines {
            println!("{}", l.as_str().unwrap_or(""));
        }
    } else {
        let mut s = String::new();
        render(&envelope, 0, &mut s);
        print!("{s}");
    }
    std::process::exit(exit);
}
