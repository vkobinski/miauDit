use miaudit_ui::{AppState, router};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "miaudit-ui", about = "Read-only local viewer for audit commits (binds to 127.0.0.1 only)")]
struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, default_value_t = 7070)]
    port: u16,
    #[arg(long)]
    audit_bin: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let a = Args::parse();
    let repo = a.repo.canonicalize()?;
    let audit_bin = a.audit_bin.unwrap_or_else(|| {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo/bin/miaudit")).unwrap_or_else(|| "miaudit".into())
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", a.port)).await?;
    println!("miauDit viewer for {} on http://127.0.0.1:{} (read-only)", repo.display(), a.port);
    axum::serve(listener, router(AppState { repo, port: a.port, audit_bin }))
        .with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; })
        .await
}
