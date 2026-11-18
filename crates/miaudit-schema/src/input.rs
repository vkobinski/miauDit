
use crate::SCHEMA_VERSION;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const MAX_INLINE_CONTENT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub code: &'static str,
    pub field: String,
    pub rule: String,
}

impl Violation {
    fn schema(field: impl Into<String>, rule: impl Into<String>) -> Self {
        Violation { code: "SCHEMA_VIOLATION", field: field.into(), rule: rule.into() }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Supports,
    Contradicts,
    Context,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Frontend,
    Api,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Before,
    After,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Local,
    Staging,
    Production,
    Ci,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLink {
    pub evidence_id: String,
    pub relation: Relation,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct ContextInput {
    pub schema_version: String,
    pub description: String,
    #[serde(default)]
    pub task_ref: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<ChangeKind>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    pub schema_version: String,
    pub question: String,
    pub choice: String,
    pub alternatives: Vec<String>,
    pub rationale: String,
    pub risks: Vec<String>,
    pub snapshot_id: String,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInput {
    pub schema_version: String,
    pub classification: Classification,
    pub description: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
}

pub trait Validate {
    fn validate(&self) -> Result<(), Violation>;
}

fn text(field: &str, v: &str, max: usize) -> Result<(), Violation> {
    if v.trim().is_empty() {
        return Err(Violation::schema(format!("/{field}"), "must not be empty"));
    }
    if v.chars().count() > max {
        return Err(Violation::schema(format!("/{field}"), format!("max {max} characters")));
    }
    Ok(())
}

fn list(field: &str, v: &[String], max_items: usize, max_len: usize) -> Result<(), Violation> {
    if v.len() > max_items {
        return Err(Violation::schema(format!("/{field}"), format!("max {max_items} items")));
    }
    for (i, s) in v.iter().enumerate() {
        text(&format!("{field}/{i}"), s, max_len)?;
    }
    Ok(())
}

impl Validate for ContextInput {
    fn validate(&self) -> Result<(), Violation> {
        text("description", &self.description, 4096)?;
        if let Some(t) = &self.task_ref {
            text("task_ref", t, 256)?;
        }
        list("constraints", &self.constraints, 32, 1024)?;
        list("source_refs", &self.source_refs, 64, 512)
    }
}

impl Validate for DecisionInput {
    fn validate(&self) -> Result<(), Violation> {
        text("question", &self.question, 512)?;
        text("choice", &self.choice, 2048)?;
        list("alternatives", &self.alternatives, 16, 1024)?;
        text("rationale", &self.rationale, 4096)?;
        list("risks", &self.risks, 32, 1024)?;
        if !self.snapshot_id.starts_with("snapshot_") {
            return Err(Violation::schema("/snapshot_id", "must be a snapshot id (snapshot_<digest>)"));
        }
        if self.evidence_links.len() > 64 {
            return Err(Violation::schema("/evidence_links", "max 64 items"));
        }
        Ok(())
    }
}

impl Validate for EvidenceInput {
    fn validate(&self) -> Result<(), Violation> {
        text("description", &self.description, 1024)?;
        match (&self.content, &self.file) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(Violation::schema("/content", "exactly one of content or file is required"));
            }
            (Some(c), None) if c.len() > MAX_INLINE_CONTENT => {
                return Err(Violation::schema("/content", format!("max {MAX_INLINE_CONTENT} bytes inline; use file")));
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn parse_input<T: DeserializeOwned + Validate>(bytes: &[u8]) -> Result<T, Violation> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| Violation::schema("", "body must be valid JSON"))?;
    let obj = value.as_object().ok_or_else(|| Violation::schema("", "body must be a JSON object"))?;
    match obj.get("schema_version").and_then(|v| v.as_str()) {
        Some(v) if v == SCHEMA_VERSION => {}
        Some(_) => return Err(Violation { code: "UNSUPPORTED_SCHEMA", field: "/schema_version".into(), rule: format!("supported version: {SCHEMA_VERSION}") }),
        None => return Err(Violation::schema("/schema_version", "required")),
    }
    let parsed: T = serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e.path().to_string();
        let field = if path == "." { String::new() } else { format!("/{}", path.replace('.', "/")) };
        Violation::schema(field, e.inner().to_string())
    })?;
    parsed.validate()?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECISION: &str = r#"{"schema_version":"1","question":"q","choice":"c","alternatives":["a"],"rationale":"r","risks":[],"snapshot_id":"snapshot_x"}"#;

    #[test]
    fn accepts_valid_decision() {
        assert!(parse_input::<DecisionInput>(DECISION.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_agent_authored_authority_fields() {
        for extra in ["approved_by", "verified", "origin"] {
            let body = DECISION.replace("\"risks\":[]", &format!("\"risks\":[],\"{extra}\":true"));
            let err = parse_input::<DecisionInput>(body.as_bytes()).unwrap_err();
            assert_eq!(err.code, "SCHEMA_VIOLATION", "{extra}");
            assert!(err.rule.contains(extra), "{}", err.rule);
        }
    }

    #[test]
    fn rejects_unsupported_version_and_bounds() {
        let err = parse_input::<DecisionInput>(DECISION.replace("\"1\"", "\"2\"").as_bytes()).unwrap_err();
        assert_eq!(err.code, "UNSUPPORTED_SCHEMA");
        let long = DECISION.replace("\"q\"", &format!("\"{}\"", "x".repeat(513)));
        assert_eq!(parse_input::<DecisionInput>(long.as_bytes()).unwrap_err().field, "/question");
    }

    #[test]
    fn new_optional_fields_do_not_change_old_serialization() {
        let ctx: ContextInput = parse_input(br#"{"schema_version":"1","description":"d"}"#).unwrap();
        assert!(!serde_json::to_string(&ctx).unwrap().contains("change_kind"));
        let ev: EvidenceInput = parse_input(br#"{"schema_version":"1","classification":"internal","description":"d","content":"x"}"#).unwrap();
        let text = serde_json::to_string(&ev).unwrap();
        assert!(!text.contains("phase") && !text.contains("environment"));
    }

    #[test]
    fn rejects_unknown_phase_environment_and_change_kind() {
        let base = r#"{"schema_version":"1","classification":"internal","description":"d","content":"x","#;
        assert!(parse_input::<EvidenceInput>(format!(r#"{base}"phase":"during"}}"#).as_bytes()).is_err());
        assert!(parse_input::<EvidenceInput>(format!(r#"{base}"environment":"laptop"}}"#).as_bytes()).is_err());
        assert!(parse_input::<EvidenceInput>(format!(r#"{base}"phase":"before","environment":"staging"}}"#).as_bytes()).is_ok());
        assert!(parse_input::<ContextInput>(br#"{"schema_version":"1","description":"d","change_kind":"mobile"}"#).is_err());
    }
}
