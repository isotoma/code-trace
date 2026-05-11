use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Source {
    #[default]
    ClaudeCode,
    Opencode,
}

impl Source {
    pub fn agent_tag(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Opencode => "opencode",
        }
    }

    pub fn version_tag_prefix(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "cc-version",
            Source::Opencode => "oc-version",
        }
    }

    pub fn trace_name_prefix(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "Claude Code",
            Source::Opencode => "OpenCode",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Opencode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude-code" | "claude_code" | "claude" => Some(Source::ClaudeCode),
            "opencode" | "open_code" => Some(Source::Opencode),
            _ => None,
        }
    }
}


