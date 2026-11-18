use miaudit_schema::input::Violation;
use serde_json::{Value, json};

pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_VALIDATION: i32 = 2;
pub const EXIT_CONFLICT: i32 = 3;
pub const EXIT_INSUFFICIENT_EVIDENCE: i32 = 4;

#[derive(Debug)]
pub struct CliError {
    pub code: &'static str,
    pub field: Option<String>,
    pub rule: String,
    pub expected: Option<String>,
    pub retryable: bool,
    pub exit: i32,
}

impl CliError {
    pub fn new(code: &'static str, exit: i32, rule: impl Into<String>) -> Self {
        CliError { code, field: None, rule: rule.into(), expected: None, retryable: false, exit }
    }
    pub fn validation(code: &'static str, rule: impl Into<String>) -> Self {
        Self::new(code, EXIT_VALIDATION, rule)
    }
    pub fn conflict(code: &'static str, rule: impl Into<String>) -> Self {
        Self::new(code, EXIT_CONFLICT, rule)
    }
    pub fn internal(rule: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", EXIT_INTERNAL, rule)
    }
    pub fn field(mut self, f: impl Into<String>) -> Self {
        self.field = Some(f.into());
        self
    }
    pub fn expected(mut self, e: impl Into<String>) -> Self {
        self.expected = Some(e.into());
        self
    }
    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "field": self.field, "rule": self.rule, "expected": self.expected, "retryable": self.retryable})
    }
}

impl From<Violation> for CliError {
    fn from(v: Violation) -> Self {
        let mut e = CliError::validation(v.code, v.rule);
        e.field = Some(v.field);
        e
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::internal(format!("io error: {e}"))
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::internal(format!("json error: {e}"))
    }
}

impl From<miaudit_schema::canonical::CanonError> for CliError {
    fn from(e: miaudit_schema::canonical::CanonError) -> Self {
        CliError::internal(e.to_string())
    }
}

pub type Res<T> = Result<T, CliError>;
