use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn newest(path: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(path).ok()?;
    if meta.is_file() {
        return meta.modified().ok();
    }
    fs::read_dir(path).ok()?.flatten().filter_map(|e| newest(&e.path())).max()
}

fn run(web: &Path, args: &[&str]) -> bool {
    Command::new("npm").args(args).current_dir(web).status().map(|s| s.success()).unwrap_or(false)
}

fn placeholder(dist: &Path, why: &str) {
    let _ = fs::create_dir_all(dist);
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>miauDit</title></head><body><h1>miauDit web UI is not built</h1><p>{why}</p><p>Run <code>npm ci &amp;&amp; npm run build</code> in <code>web/</code>, then rebuild this crate.</p></body></html>"
    );
    let _ = fs::write(dist.join("index.html"), html);
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let web = manifest.join("../../web");
    let dist = web.join("dist");
    let index = dist.join("index.html");
    for p in ["src", "public", "index.html", "package.json", "package-lock.json", "vite.config.ts", "tsconfig.json"] {
        println!("cargo:rerun-if-changed={}", web.join(p).display());
    }
    println!("cargo:rerun-if-env-changed=MIAUDIT_SKIP_WEB_BUILD");

    let sources = ["src", "public", "index.html", "package.json", "package-lock.json", "vite.config.ts", "tsconfig.json"].iter().filter_map(|p| newest(&web.join(p))).max();
    let built = fs::metadata(&index).and_then(|m| m.modified()).ok();
    let fresh = matches!((built, sources), (Some(b), Some(s)) if b >= s);
    let skip = env::var_os("MIAUDIT_SKIP_WEB_BUILD").is_some();

    if !fresh && !skip {
        let installed = web.join("node_modules").is_dir() || run(&web, &["ci"]);
        let built_ok = installed && run(&web, &["run", "build"]);
        if !built_ok && !index.exists() {
            println!("cargo:warning=the web UI could not be built (is Node.js installed?); embedding a placeholder page");
            placeholder(&dist, "The build needs Node.js and npm.");
        }
    } else if !index.exists() {
        placeholder(&dist, "The web build was skipped with MIAUDIT_SKIP_WEB_BUILD.");
    }

    let stamp = fs::metadata(&index).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    println!("cargo:rustc-env=MIAUDIT_WEB_STAMP={stamp}");
}
