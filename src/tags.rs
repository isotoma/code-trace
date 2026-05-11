use std::process::Command;

use crate::source::Source;

fn git_cmd(args: &[&str], cwd: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

pub fn gather_env_tags(source: Source, cwd: Option<&str>, agent_version: Option<&str>) -> Vec<String> {
    let mut tags = vec![source.agent_tag().to_string()];

    if let Some(ver) = agent_version {
        if !ver.is_empty() {
            tags.push(format!("{}:{ver}", source.version_tag_prefix()));
        }
    }

    // Git repo name
    if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
        if let Some(name) = std::path::Path::new(&toplevel).file_name() {
            tags.push(format!("repo:{}", name.to_string_lossy()));
        }
    }

    // Git branch
    if let Some(branch) = git_cmd(&["rev-parse", "--abbrev-ref", "HEAD"], cwd) {
        tags.push(format!("branch:{branch}"));
    }

    // Username
    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        tags.push(format!("user:{user}"));
    }

    // Hostname
    if let Ok(host) = hostname::get() {
        tags.push(format!("host:{}", host.to_string_lossy()));
    }

    // OS
    tags.push(format!("os:{}", std::env::consts::OS));

    // Claude Code version probe (only for Claude Code source, if no version provided)
    if source == Source::ClaudeCode && agent_version.is_none() {
        if let Ok(output) = Command::new("claude").arg("--version").output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ver.is_empty() {
                    tags.push(format!("cc-version:{ver}"));
                }
            }
        }
    }

    // OpenCode version probe (only for OpenCode source, if no version provided)
    if source == Source::Opencode && agent_version.is_none() {
        if let Ok(output) = Command::new("opencode").arg("--version").output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ver.is_empty() {
                    tags.push(format!("oc-version:{ver}"));
                }
            }
        }
    }

    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_agent_tag() {
        let tags = gather_env_tags(Source::ClaudeCode, None, None);
        assert!(tags.contains(&"claude-code".to_string()));
    }

    #[test]
    fn includes_os_tag() {
        let tags = gather_env_tags(Source::ClaudeCode, None, None);
        assert!(tags.iter().any(|t| t.starts_with("os:")));
    }

    #[test]
    fn includes_user_tag() {
        let tags = gather_env_tags(Source::ClaudeCode, None, None);
        assert!(tags.iter().any(|t| t.starts_with("user:")));
    }

    #[test]
    fn opencode_source_uses_opencode_tag() {
        let tags = gather_env_tags(Source::Opencode, None, Some("0.4.5"));
        assert!(tags.contains(&"opencode".to_string()));
        assert!(tags.contains(&"oc-version:0.4.5".to_string()));
        assert!(!tags.iter().any(|t| t.starts_with("cc-version:")));
    }

    #[test]
    fn version_from_payload_preferred_over_probe() {
        let tags = gather_env_tags(Source::Opencode, None, Some("1.2.3"));
        assert!(tags.contains(&"oc-version:1.2.3".to_string()));
    }
}
