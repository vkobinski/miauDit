
use crate::error::{CliError, Res};
use miaudit_schema::{digest::content_id, model::{EntryKind, TreeEntry}};
use std::{
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path, PathBuf},
};

pub const EXCLUDED_DIRS: [&str; 2] = [".git", ".audit"];
pub const MODE_FILE: u32 = 0o100644;
pub const MODE_EXEC: u32 = 0o100755;
pub const MODE_SYMLINK: u32 = 0o120000;

fn bad_path(rule: impl Into<String>) -> CliError {
    CliError::validation("UNSUPPORTED_PATH", rule)
}

fn check_component(c: &str) -> Res<()> {
    if c.is_empty() || c.chars().any(|ch| ch.is_control() || ch == '\\') {
        return Err(bad_path(format!("ambiguous path component {c:?}")));
    }
    Ok(())
}

pub fn resolve(root: &Path, cwd: &Path, arg: &str) -> Res<(PathBuf, String)> {
    if arg.contains('\0') {
        return Err(bad_path("NUL in path"));
    }
    let p = Path::new(arg);
    let joined = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    let mut norm = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                norm.pop();
            }
            Component::CurDir => {}
            other => norm.push(other.as_os_str()),
        }
    }
    let rel = norm.strip_prefix(root).map_err(|_| bad_path("path escapes the repository root"))?;
    let mut parts = Vec::new();
    for c in rel.components() {
        let s = c.as_os_str().to_str().ok_or_else(|| bad_path("only UTF-8 paths are supported"))?;
        check_component(s)?;
        if EXCLUDED_DIRS.contains(&s) {
            return Err(bad_path(format!("{s} is excluded from capture")));
        }
        parts.push(s.to_string());
    }
    if let (false, Some(parent)) = (parts.is_empty(), norm.parent())
        && let Ok(real) = parent.canonicalize()
        && !real.starts_with(root)
    {
        return Err(bad_path("path escapes the repository root through a symlink"));
    }
    Ok((norm, parts.join("/")))
}

pub fn capture(abs: &Path, rel: &str) -> Res<(TreeEntry, Vec<u8>)> {
    let m1 = fs::symlink_metadata(abs)?;
    if m1.file_type().is_symlink() {
        let target = fs::read_link(abs)?;
        let bytes = target.as_os_str().as_bytes().to_vec();
        let entry = TreeEntry { path: rel.into(), kind: EntryKind::Symlink, mode: MODE_SYMLINK, digest: content_id(&bytes), size: bytes.len() as u64 };
        return Ok((entry, bytes));
    }
    if !m1.is_file() {
        return Err(bad_path(format!("{rel}: only regular files and symlinks are supported")));
    }
    let bytes = fs::read(abs)?;
    let m2 = fs::symlink_metadata(abs)?;
    if m1.len() != m2.len() || m1.modified()? != m2.modified()? || bytes.len() as u64 != m1.len() {
        return Err(CliError::new("CAPTURE_INDETERMINATE", crate::error::EXIT_CONFLICT, format!("{rel} changed during capture; retry")));
    }
    let mode = if m1.permissions().mode() & 0o111 != 0 { MODE_EXEC } else { MODE_FILE };
    let entry = TreeEntry { path: rel.into(), kind: EntryKind::File, mode, digest: content_id(&bytes), size: bytes.len() as u64 };
    Ok((entry, bytes))
}

pub fn walk(abs_dir: &Path, rel_prefix: &str, out: &mut Vec<(PathBuf, String)>) -> Res<()> {
    let mut names: Vec<_> = fs::read_dir(abs_dir)?.collect::<Result<Vec<_>, _>>()?;
    names.sort_by_key(|e| e.file_name());
    for e in names {
        let name = e.file_name();
        let name = name.to_str().ok_or_else(|| bad_path("only UTF-8 paths are supported"))?.to_string();
        check_component(&name)?;
        let ft = e.file_type()?;
        let rel = if rel_prefix.is_empty() { name.clone() } else { format!("{rel_prefix}/{name}") };
        if ft.is_dir() {
            if EXCLUDED_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&e.path(), &rel, out)?;
        } else if ft.is_symlink() || ft.is_file() {
            out.push((e.path(), rel));
        } else {
            return Err(bad_path(format!("{rel}: only regular files and symlinks are supported")));
        }
    }
    Ok(())
}
