use crate::error::{CliError, Res};
use std::path::Path;

#[cfg(feature = "ui")]
pub fn serve(cwd: &Path, repo: Option<&Path>, port: u16, json_out: bool) -> Res<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let start = match repo {
        Some(p) => p.canonicalize().map_err(|e| CliError::validation("BAD_REPO", format!("{}: {e}", p.display())).field("/repo"))?,
        None => cwd.to_path_buf(),
    };
    let root = crate::store::Store::discover(&start)?.root;
    let audit_bin = std::env::current_exe().map_err(|e| CliError::internal(format!("cannot locate this executable: {e}")))?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| CliError::internal(format!("cannot start the async runtime: {e}")))?;
    let slug = miaudit_ui::local_slug(&root);
    let ready = Arc::new(AtomicBool::new(false));
    let flag = ready.clone();
    let shown = root.clone();
    let result = runtime.block_on(miaudit_ui::serve(root, port, audit_bin, move |addr| {
        flag.store(true, Ordering::SeqCst);
        let url = format!("http://{addr}");
        if json_out {
            println!("{}", serde_json::json!({"contract_version": crate::CONTRACT_VERSION, "ok": true, "result": {"url": url, "port": addr.port(), "repo": shown, "slug": slug}}));
        } else {
            println!("miauDit viewer for {} on {url} (read-only, Ctrl-C to stop)", shown.display());
        }
    }));
    result.map_err(|e| {
        if ready.load(Ordering::SeqCst) {
            CliError::internal(format!("the viewer stopped: {e}"))
        } else {
            CliError::validation("UI_BIND_FAILED", format!("cannot listen on 127.0.0.1:{port}: {e}")).field("/port")
        }
    })
}

#[cfg(not(feature = "ui"))]
pub fn serve(_cwd: &Path, _repo: Option<&Path>, _port: u16, _json_out: bool) -> Res<()> {
    Err(CliError::validation("UI_NOT_BUILT", "this miaudit was built without the ui feature; reinstall with `cargo install --path crates/miaudit-cli --locked`"))
}
