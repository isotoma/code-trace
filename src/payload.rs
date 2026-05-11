use serde_json::Value;
use std::path::PathBuf;

use crate::source::Source;

pub struct HookPayload {
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub cwd: Option<String>,
}

pub enum Input {
    ClaudeCode {
        session_id: Option<String>,
        transcript_path: Option<PathBuf>,
        cwd: Option<String>,
    },
    Opencode {
        session_id: Option<String>,
        cwd: Option<String>,
        messages: Vec<Value>,
        agent_version: Option<String>,
    },
    PiAgent {
        session_id: Option<String>,
        cwd: Option<String>,
        messages: Vec<Value>,
        agent_version: Option<String>,
    },
}

impl Input {
    pub fn source(&self) -> Source {
        match self {
            Input::ClaudeCode { .. } => Source::ClaudeCode,
            Input::Opencode { .. } => Source::Opencode,
            Input::PiAgent { .. } => Source::PiAgent,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Input::ClaudeCode { session_id, .. } => session_id.as_deref(),
            Input::Opencode { session_id, .. } => session_id.as_deref(),
            Input::PiAgent { session_id, .. } => session_id.as_deref(),
        }
    }

    pub fn cwd(&self) -> Option<&str> {
        match self {
            Input::ClaudeCode { cwd, .. } => cwd.as_deref(),
            Input::Opencode { cwd, .. } => cwd.as_deref(),
            Input::PiAgent { cwd, .. } => cwd.as_deref(),
        }
    }

    pub fn agent_version(&self) -> Option<&str> {
        match self {
            Input::ClaudeCode { .. } => None,
            Input::Opencode { agent_version, .. } => agent_version.as_deref(),
            Input::PiAgent { agent_version, .. } => agent_version.as_deref(),
        }
    }
}

pub fn parse_payload(value: &Value) -> Input {
    if let Some(source_val) = value.get("source").and_then(|v| v.as_str()) {
        if let Some(source) = Source::parse(source_val) {
            return parse_by_source(&source, value);
        }
    }
    parse_claude_code_payload(value)
}

fn parse_by_source(source: &Source, value: &Value) -> Input {
    match source {
        Source::ClaudeCode => parse_claude_code_payload(value),
        Source::Opencode => parse_opencode_payload(value),
        Source::PiAgent => parse_pi_agent_payload(value),
    }
}

fn parse_claude_code_payload(value: &Value) -> Input {
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

    let cwd = value.get("cwd").and_then(|v| v.as_str()).map(String::from);

    Input::ClaudeCode {
        session_id,
        transcript_path: transcript,
        cwd,
    }
}

fn parse_opencode_payload(value: &Value) -> Input {
    let session_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let cwd = value.get("cwd").and_then(|v| v.as_str()).map(String::from);

    let messages = value
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let agent_version = value
        .get("agentVersion")
        .or_else(|| value.get("agent_version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Input::Opencode {
        session_id,
        cwd,
        messages,
        agent_version,
    }
}

fn parse_pi_agent_payload(value: &Value) -> Input {
    let session_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let cwd = value.get("cwd").and_then(|v| v.as_str()).map(String::from);

    let messages = value
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let agent_version = value
        .get("agentVersion")
        .or_else(|| value.get("agent_version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Input::PiAgent {
        session_id,
        cwd,
        messages,
        agent_version,
    }
}

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
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::ClaudeCode);
        assert_eq!(input.session_id(), Some("abc-123"));
    }

    #[test]
    fn parses_snake_case_payload() {
        let v = json!({
            "session_id": "abc-123",
            "transcript_path": "/tmp/test.jsonl"
        });
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::ClaudeCode);
        assert_eq!(input.session_id(), Some("abc-123"));
    }

    #[test]
    fn parses_explicit_claude_code_source() {
        let v = json!({
            "source": "claude-code",
            "sessionId": "abc-123",
            "transcriptPath": "/tmp/test.jsonl"
        });
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::ClaudeCode);
    }

    #[test]
    fn parses_opencode_payload() {
        let v = json!({
            "source": "opencode",
            "sessionId": "ses_123",
            "cwd": "/home/user/project",
            "messages": [{"type": "user", "content": "hello"}],
            "agentVersion": "0.4.5"
        });
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::Opencode);
        assert_eq!(input.session_id(), Some("ses_123"));
        assert_eq!(input.agent_version(), Some("0.4.5"));
        match &input {
            Input::Opencode { messages, .. } => assert_eq!(messages.len(), 1),
            _ => panic!("expected Opencode input"),
        }
    }

    #[test]
    fn parses_pi_agent_payload() {
        let v = json!({
            "source": "pi-agent",
            "sessionId": "ses_456",
            "cwd": "/home/user/project",
            "messages": [{"role": "user", "content": "hello"}],
            "agentVersion": "1.0.0"
        });
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::PiAgent);
        assert_eq!(input.session_id(), Some("ses_456"));
        assert_eq!(input.agent_version(), Some("1.0.0"));
        match &input {
            Input::PiAgent { messages, .. } => assert_eq!(messages.len(), 1),
            _ => panic!("expected PiAgent input"),
        }
    }

    #[test]
    fn defaults_to_claude_code_when_source_missing() {
        let v = json!({"sessionId": "abc"});
        let input = parse_payload(&v);
        assert_eq!(input.source(), Source::ClaudeCode);
    }

    #[test]
    fn handles_empty_payload() {
        let input = parse_payload(&Value::Null);
        assert_eq!(input.source(), Source::ClaudeCode);
        assert_eq!(input.session_id(), None);
    }
}
