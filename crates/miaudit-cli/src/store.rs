
use crate::error::{CliError, Res};
use miaudit_schema::{
    digest::content_id,
    model::{EvidenceBody, Manifest, Stored, StoredBinding, TreeEntry},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, fs, io::Write, path::{Path, PathBuf}};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub schema_version: String,
    pub repo_id: String,
    pub created_at: String,
    pub case_sensitive: Option<bool>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Index {
    pub staged: BTreeMap<String, Option<TreeEntry>>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Pending {
    pub contexts: Vec<String>,
    pub decisions: Vec<String>,
    pub evidence: Vec<String>,
}

pub struct Store {
    pub root: PathBuf,
    pub dir: PathBuf,
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Res<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Res<bool> {
    match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => {
            f.write_all(bytes)?;
            f.sync_all()?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(path)? == bytes {
                Ok(false)
            } else {
                Err(CliError::conflict("INTEGRITY_FAILURE", format!("immutable object {} exists with different content", path.display())))
            }
        }
        Err(e) => Err(e.into()),
    }
}

impl Store {
    pub fn init(root: &Path, config: &Config) -> Res<Store> {
        let dir = root.join(".miaudit");
        for d in ["", "objects/contexts", "objects/decisions", "objects/evidence", "blobs", "commits"] {
            fs::create_dir_all(dir.join(d))?;
        }
        write_atomic(&dir.join("config.json"), &serde_json::to_vec_pretty(config)?)?;
        write_atomic(&dir.join(".gitignore"), b"*\n")?;
        Ok(Store { root: root.to_path_buf(), dir })
    }

    pub fn discover(cwd: &Path) -> Res<Store> {
        let mut cur = Some(cwd);
        while let Some(d) = cur {
            if d.join(".miaudit/config.json").is_file() {
                return Ok(Store { root: d.to_path_buf(), dir: d.join(".miaudit") });
            }
            if d.join(".audit/config.json").is_file() {
                return Err(CliError::validation("LEGACY_STORE", format!("{} holds a store in the old .audit/ directory; rename it with `mv .audit .miaudit`", d.display())));
            }
            cur = d.parent();
        }
        Err(CliError::validation("NOT_INITIALIZED", "no .miaudit directory found; run `miaudit init`"))
    }

    fn read_json<T: DeserializeOwned>(&self, rel: &str) -> Res<Option<T>> {
        match fs::read(self.dir.join(rel)) {
            Ok(b) => Ok(Some(serde_json::from_slice(&b)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn config(&self) -> Res<Config> {
        self.read_json("config.json")?.ok_or_else(|| CliError::validation("NOT_INITIALIZED", "missing config"))
    }
    pub fn index(&self) -> Res<Index> {
        Ok(self.read_json("index.json")?.unwrap_or_default())
    }
    pub fn save_index(&self, i: &Index) -> Res<()> {
        write_atomic(&self.dir.join("index.json"), &serde_json::to_vec_pretty(i)?)
    }
    pub fn pending(&self) -> Res<Pending> {
        Ok(self.read_json("pending.json")?.unwrap_or_default())
    }
    pub fn save_pending(&self, p: &Pending) -> Res<()> {
        write_atomic(&self.dir.join("pending.json"), &serde_json::to_vec_pretty(p)?)
    }
    pub fn head(&self) -> Res<Option<String>> {
        match fs::read_to_string(self.dir.join("HEAD")) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn set_head(&self, id: &str) -> Res<()> {
        write_atomic(&self.dir.join("HEAD"), id.as_bytes())
    }

    pub fn put_blob(&self, bytes: &[u8]) -> Res<String> {
        let id = content_id(bytes);
        let hex = id.trim_start_matches("sha256:");
        write_immutable(&self.dir.join("blobs").join(hex), bytes)?;
        Ok(id)
    }
    pub fn get_blob(&self, digest: &str) -> Res<Option<Vec<u8>>> {
        let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        match fs::read(self.dir.join("blobs").join(hex)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn object_dir(id: &str) -> Option<&'static str> {
        match id.split('_').next() {
            Some("ctx") => Some("objects/contexts"),
            Some("dec") => Some("objects/decisions"),
            Some("evd") => Some("objects/evidence"),
            _ => None,
        }
    }

    pub fn put_object(&self, id: &str, bytes: &[u8]) -> Res<bool> {
        let d = Self::object_dir(id).ok_or_else(|| CliError::internal("bad object id"))?;
        write_immutable(&self.dir.join(d).join(format!("{id}.json")), bytes)
    }
    pub fn get_object_bytes(&self, id: &str) -> Res<Option<Vec<u8>>> {
        let Some(d) = Self::object_dir(id) else { return Ok(None) };
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Ok(None);
        }
        match fs::read(self.dir.join(d).join(format!("{id}.json"))) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn get_object<T: DeserializeOwned>(&self, id: &str) -> Res<Option<Stored<T>>> {
        match self.get_object_bytes(id)? {
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
            None => Ok(None),
        }
    }
    pub fn get_evidence(&self, id: &str) -> Res<Option<Stored<EvidenceBody>>> {
        self.get_object(id)
    }

    pub fn put_commit(&self, id: &str, bytes: &[u8]) -> Res<bool> {
        write_immutable(&self.dir.join("commits").join(format!("{id}.json")), bytes)
    }
    pub fn get_commit_bytes(&self, id: &str) -> Res<Option<Vec<u8>>> {
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Ok(None);
        }
        match fs::read(self.dir.join("commits").join(format!("{id}.json"))) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn get_commit(&self, id: &str) -> Res<Option<Manifest>> {
        match self.get_commit_bytes(id)? {
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
            None => Ok(None),
        }
    }

    pub fn put_binding(&self, id: &str, bytes: &[u8]) -> Res<bool> {
        fs::create_dir_all(self.dir.join("bindings"))?;
        let path = self.dir.join("bindings").join(format!("{id}.json"));
        if path.exists() {
            return Ok(false);
        }
        write_immutable(&path, bytes)
    }
    pub fn list_bindings(&self) -> Res<Vec<StoredBinding>> {
        let dir = self.dir.join("bindings");
        let Ok(rd) = fs::read_dir(&dir) else { return Ok(vec![]) };
        let mut out = Vec::new();
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                out.push(serde_json::from_slice(&fs::read(e.path())?)?);
            }
        }
        out.sort_by(|a: &StoredBinding, b| (a.created.at.as_str(), a.binding_id.as_str()).cmp(&(b.created.at.as_str(), b.binding_id.as_str())));
        Ok(out)
    }
}
