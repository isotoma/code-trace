use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Source {
    #[default]
    ClaudeCode,
    Opencode,
    PiAgent,
}

impl Source {
    pub fn agent_tag(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Opencode => "opencode",
            Source::PiAgent => "pi-agent",
        }
    }

    pub fn version_tag_prefix(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "cc-version",
            Source::Opencode => "oc-version",
            Source::PiAgent => "pi-version",
        }
    }

    pub fn trace_name_prefix(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "Claude Code",
            Source::Opencode => "OpenCode",
            Source::PiAgent => "Pi Agent",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Opencode => "opencode",
            Source::PiAgent => "pi-agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude-code" | "claude_code" | "claude" => Some(Source::ClaudeCode),
            "opencode" | "open_code" => Some(Source::Opencode),
            "pi-agent" | "pi_agent" | "pi" => Some(Source::PiAgent),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_sources() {
        assert_eq!(Source::parse("claude-code"), Some(Source::ClaudeCode));
        assert_eq!(Source::parse("opencode"), Some(Source::Opencode));
        assert_eq!(Source::parse("pi-agent"), Some(Source::PiAgent));
    }

    // SessionStart payloads carry a `source` field of "startup"/"resume"/etc.
    // These must NOT parse as an agent source, so the payload falls through
    // to Claude Code parsing.
    #[test]
    fn rejects_session_start_source_values() {
        for v in ["startup", "resume", "clear", "compact", "unknown"] {
            assert_eq!(Source::parse(v), None, "{v} must not parse as a Source");
        }
    }
}
