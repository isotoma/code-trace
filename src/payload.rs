use serde_json::Value;
use std::path::PathBuf;

pub struct HookPayload {
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub cwd: Option<String>,
}

/// Parse hook payload from a JSON value. Tolerates multiple field name conventions.
pub fn parse_payload(value: &Value) -> HookPayload {
    let session_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let transcript = value
        .get("transcriptPath")
        .or_else(|| value.get("transcript_path"))
        .and_then(|v| v.as_str())
        .map(|s| {
            let p = PathBuf::from(s);
            if s.starts_with('~') {
                if let Some(home) = dirs::home_dir() {
                    return home.join(s.strip_prefix("~/").unwrap_or(s));
                }
            }
            p
        });

    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(String::from);

    HookPayload {
        session_id,
        transcript_path: transcript,
        cwd,
    }
}

/// Read stdin fully and parse as JSON. Returns Value::Null on any failure.
pub fn read_stdin() -> Value {
    let mut buf = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).is_err() {
        return Value::Null;
    }
    serde_json::from_str(&buf).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_camel_case_payload() {
        let v = json!({
            "sessionId": "abc-123",
            "transcriptPath": "/tmp/test.jsonl",
            "cwd": "/home/user/project"
        });
        let p = parse_payload(&v);
        assert_eq!(p.session_id.as_deref(), Some("abc-123"));
        assert_eq!(p.transcript_path.as_deref(), Some(std::path::Path::new("/tmp/test.jsonl")));
        assert_eq!(p.cwd.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn parses_snake_case_payload() {
        let v = json!({
            "session_id": "abc-123",
            "transcript_path": "/tmp/test.jsonl"
        });
        let p = parse_payload(&v);
        assert_eq!(p.session_id.as_deref(), Some("abc-123"));
        assert_eq!(p.transcript_path.as_deref(), Some(std::path::Path::new("/tmp/test.jsonl")));
    }

    #[test]
    fn handles_empty_payload() {
        let p = parse_payload(&Value::Null);
        assert!(p.session_id.is_none());
        assert!(p.transcript_path.is_none());
    }
}
