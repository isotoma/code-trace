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

/// Whether `cwd` (or the process working directory when `None`) is inside a
/// git work tree. Any subdirectory of a repo counts — `git rev-parse` walks up
/// to the repo root. A missing `git` binary or any error is treated as "not a
/// repo".
pub fn cwd_in_git_repo(cwd: Option<&str>) -> bool {
    git_cmd(&["rev-parse", "--is-inside-work-tree"], cwd).as_deref() == Some("true")
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

    // Pi Agent version probe (only for PiAgent source, if no version provided)
    if source == Source::PiAgent && agent_version.is_none() {
        if let Ok(output) = Command::new("pi").arg("--version").output() {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ver.is_empty() {
                    tags.push(format!("pi-version:{ver}"));
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
    fn cwd_in_git_repo_true_for_repo_and_false_for_plain_dir() {
        let repo = tempfile::TempDir::new().unwrap();
        let repo_path = repo.path().to_string_lossy().to_string();
        // git init makes it a work tree; a subdir must also count.
        assert!(git_cmd(&["init"], Some(&repo_path)).is_some());
        assert!(cwd_in_git_repo(Some(&repo_path)));
        let sub = repo.path().join("nested/deeper");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(cwd_in_git_repo(Some(&sub.to_string_lossy())));

        let plain = tempfile::TempDir::new().unwrap();
        assert!(!cwd_in_git_repo(Some(&plain.path().to_string_lossy())));
    }

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

    #[test]
    fn pi_agent_source_uses_pi_agent_tag() {
        let tags = gather_env_tags(Source::PiAgent, None, Some("1.0.0"));
        assert!(tags.contains(&"pi-agent".to_string()));
        assert!(tags.contains(&"pi-version:1.0.0".to_string()));
        assert!(!tags.iter().any(|t| t.starts_with("cc-version:")));
        assert!(!tags.iter().any(|t| t.starts_with("oc-version:")));
    }
}
