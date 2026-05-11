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
