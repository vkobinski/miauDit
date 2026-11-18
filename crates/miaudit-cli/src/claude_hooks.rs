use crate::error::{CliError, Res};
use crate::store::write_atomic;
use serde_json::{Map, Value};
use std::{fs, path::{Path, PathBuf}};

const TEMPLATE: &str = include_str!("../hooks/claude-code.settings.json");

pub struct Plan {
    pub path: PathBuf,
    pub created_file: bool,
    pub added: Vec<String>,
    pub already_present: Vec<String>,
    merged: Option<Value>,
}

fn invalid(path: &Path, why: &str) -> CliError {
    CliError::validation("HOOKS_SETTINGS_INVALID", format!("{}: {why}; fix or remove it, nothing was changed", path.display()))
}

fn commands(entry: &Value) -> Vec<&str> {
    entry["hooks"].as_array().into_iter().flatten().filter_map(|h| h["command"].as_str()).collect()
}

pub fn plan(root: &Path, local: bool) -> Res<Plan> {
    let path = root.join(".claude").join(if local { "settings.local.json" } else { "settings.json" });
    let template: Value = serde_json::from_str(TEMPLATE).map_err(|e| CliError::internal(format!("bundled hooks template is invalid: {e}")))?;
    let (mut settings, created_file) = match fs::read(&path) {
        Ok(bytes) => (serde_json::from_slice::<Value>(&bytes).map_err(|e| invalid(&path, &format!("not valid JSON ({e})")))?, false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Value::Object(Map::new()), true),
        Err(e) => return Err(e.into()),
    };
    let Some(top) = settings.as_object_mut() else { return Err(invalid(&path, "the top level is not a JSON object")) };
    let hooks = top.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks) = hooks.as_object_mut() else { return Err(invalid(&path, "\"hooks\" is not an object")) };
    let (mut added, mut already_present) = (Vec::new(), Vec::new());
    for (event, entries) in template["hooks"].as_object().into_iter().flatten() {
        let slot = hooks.entry(event.clone()).or_insert_with(|| Value::Array(vec![]));
        let Some(existing) = slot.as_array_mut() else { return Err(invalid(&path, &format!("hooks.{event} is not an array"))) };
        let mut changed = false;
        for entry in entries.as_array().into_iter().flatten() {
            let wanted = commands(entry);
            let present = existing.iter().any(|e| commands(e).iter().any(|c| wanted.contains(c)));
            if !present {
                existing.push(entry.clone());
                changed = true;
            }
        }
        if changed { added.push(event.clone()) } else { already_present.push(event.clone()) }
    }
    let merged = (!added.is_empty()).then_some(settings);
    Ok(Plan { path, created_file, added, already_present, merged })
}

impl Plan {
    pub fn apply(&self) -> Res<()> {
        let Some(merged) = &self.merged else { return Ok(()) };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string_pretty(merged)?;
        text.push('\n');
        write_atomic(&self.path, text.as_bytes())
    }
}

pub fn binary_warning() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let target = Path::new(&home).join(".cargo/bin/miaudit");
    (!target.exists()).then(|| "the hooks run $HOME/.cargo/bin/miaudit, which does not exist here; install it with `cargo install --path crates/miaudit-cli --locked`".to_string())
}
