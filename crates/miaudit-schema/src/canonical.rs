
use serde::Serialize;
use serde_json::Value;

pub const C14N_VERSION: &str = "auditai-c14n-v1";

#[derive(Debug, PartialEq, Eq)]
pub enum CanonError {
    NonInteger,
    Serialize(String),
}

impl std::fmt::Display for CanonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanonError::NonInteger => write!(f, "non-integer number is not allowed in canonical form"),
            CanonError::Serialize(e) => write!(f, "serialization failed: {e}"),
        }
    }
}

pub fn to_canonical_bytes<T: Serialize>(v: &T) -> Result<Vec<u8>, CanonError> {
    let value = serde_json::to_value(v).map_err(|e| CanonError::Serialize(e.to_string()))?;
    let mut out = Vec::new();
    write_value(&value, &mut out)?;
    Ok(out)
}

fn write_value(v: &Value, out: &mut Vec<u8>) -> Result<(), CanonError> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(i.to_string().as_bytes());
            } else if let Some(u) = n.as_u64() {
                out.extend_from_slice(u.to_string().as_bytes());
            } else {
                return Err(CanonError::NonInteger);
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(a) => {
            out.push(b'[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(k, out);
                out.push(b':');
                write_value(&m[*k], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes()),
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(v: Value) -> String {
        String::from_utf8(to_canonical_bytes(&v).unwrap()).unwrap()
    }

    #[test]
    fn sorts_keys_and_strips_whitespace() {
        assert_eq!(c(json!({"b": 1, "a": [true, null, "x"]})), r#"{"a":[true,null,"x"],"b":1}"#);
    }

    #[test]
    fn orders_keys_by_utf16_code_units() {
        assert_eq!(c(json!({"\u{ff5e}": 1, "\u{1f600}": 2})), "{\"\u{1f600}\":2,\"\u{ff5e}\":1}");
    }

    #[test]
    fn escapes_minimally_and_keeps_unicode() {
        assert_eq!(c(json!("a\"b\\c\n\u{1}é")), "\"a\\\"b\\\\c\\n\\u0001é\"");
    }

    #[test]
    fn rejects_floats() {
        assert_eq!(to_canonical_bytes(&json!({"x": 1.5})), Err(CanonError::NonInteger));
        assert_eq!(to_canonical_bytes(&json!({"x": 1.0})), Err(CanonError::NonInteger));
    }

    #[test]
    fn handles_integer_extremes() {
        assert_eq!(c(json!([i64::MIN, u64::MAX])), "[-9223372036854775808,18446744073709551615]");
    }
}
