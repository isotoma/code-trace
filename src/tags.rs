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

/// Extract the owning organisation (or user) from a git remote URL.
///
/// Handles the common forms:
/// - `git@github.com:orgname/reponame.git`
/// - `ssh://git@github.com/orgname/reponame.git`
/// - `https://github.com/orgname/reponame.git`
/// - `https://github.com/orgname/reponame`
///
/// The organisation is the path segment immediately preceding the repo name.
/// Returns `None` for URLs we can't parse (e.g. local paths).
fn org_from_remote_url(url: &str) -> Option<String> {
    // Strip a trailing `.git` so the repo name segment is clean.
    let url = url.trim().trim_end_matches(".git");

    // Normalise the SCP-style `host:org/repo` into `host/org/repo`.
    let path = match url.split_once("://") {
        // ssh://git@github.com/org/repo or https://github.com/org/repo
        Some((_, rest)) => {
            let after_host = rest.split_once('/').map(|(_, h)| h).unwrap_or(rest);
            after_host
        }
        None => match url.split_once(':') {
            // git@github.com:org/repo
            Some((_, rest)) => rest,
            None => return None, // local path or unparseable
        },
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Need at least `org/repo`.
    if segments.len() < 2 {
        return None;
    }
    let org = segments[segments.len() - 2];
    if org.is_empty() {
        None
    } else {
        Some(org.to_string())
    }
}

/// Extract the repository name from a git remote URL.
///
/// Handles SCP-style (`git@github.com:org/repo.git`),
/// ssh:// (`ssh://git@github.com/org/repo.git`),
/// and https:// (`https://github.com/org/repo.git`) forms.
/// Returns `None` for local paths or unparseable URLs.
fn repo_from_remote_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(".git");

    let path = match url.split_once("://") {
        Some((_, rest)) => {
            let after_host = rest.split_once('/').map(|(_, h)| h).unwrap_or(rest);
            after_host
        }
        None => match url.split_once(':') {
            Some((_, rest)) => rest,
            None => return None,
        },
    };

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    let repo = segments[segments.len() - 1];
    if repo.is_empty() {
        None
    } else {
        Some(repo.to_string())
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

    // Git organisation (owner) from the origin remote URL.
    if let Some(url) = git_cmd(&["remote", "get-url", "origin"], cwd) {
        if let Some(org) = org_from_remote_url(&url) {
            tags.push(format!("org:{org}"));
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

    #[test]
    fn org_from_ssh_url() {
        assert_eq!(
            org_from_remote_url("git@github.com:acme/widgets.git"),
            Some("acme".to_string())
        );
    }

    #[test]
    fn org_from_https_url() {
        assert_eq!(
            org_from_remote_url("https://github.com/acme/widgets.git"),
            Some("acme".to_string())
        );
    }

    #[test]
    fn org_from_ssh_protocol_url() {
        assert_eq!(
            org_from_remote_url("ssh://git@github.com/acme/widgets.git"),
            Some("acme".to_string())
        );
    }

    #[test]
    fn org_from_https_url_without_git_suffix() {
        assert_eq!(
            org_from_remote_url("https://github.com/acme/widgets"),
            Some("acme".to_string())
        );
    }

    #[test]
    fn org_from_gitlab_subgroup_url() {
        assert_eq!(
            org_from_remote_url("git@gitlab.com:acme/platform/widgets.git"),
            Some("platform".to_string())
        );
    }

    #[test]
    fn org_from_local_path_returns_none() {
        assert_eq!(org_from_remote_url("/home/doug/projects/widgets"), None);
    }

    #[test]
    fn repo_from_ssh_url() {
        assert_eq!(
            repo_from_remote_url("git@github.com:acme/widgets.git"),
            Some("widgets".to_string())
        );
    }

    #[test]
    fn repo_from_https_url() {
        assert_eq!(
            repo_from_remote_url("https://github.com/acme/widgets.git"),
            Some("widgets".to_string())
        );
    }

    #[test]
    fn repo_from_ssh_protocol_url() {
        assert_eq!(
            repo_from_remote_url("ssh://git@github.com/acme/widgets.git"),
            Some("widgets".to_string())
        );
    }

    #[test]
    fn repo_from_https_url_without_git_suffix() {
        assert_eq!(
            repo_from_remote_url("https://github.com/acme/widgets"),
            Some("widgets".to_string())
        );
    }

    #[test]
    fn repo_from_gitlab_subgroup_url() {
        assert_eq!(
            repo_from_remote_url("git@gitlab.com:acme/platform/widgets.git"),
            Some("widgets".to_string())
        );
    }

    #[test]
    fn repo_from_local_path_returns_none() {
        assert_eq!(repo_from_remote_url("/home/doug/projects/widgets"), None);
    }

    #[test]
    fn org_tag_emitted_when_remote_present() {
        let repo = tempfile::TempDir::new().unwrap();
        let repo_path = repo.path().to_string_lossy().to_string();
        assert!(git_cmd(&["init"], Some(&repo_path)).is_some());
        // `git remote add` produces no stdout, so check exit status directly.
        let status = Command::new("git")
            .args(["remote", "add", "origin", "git@github.com:acme/widgets.git"])
            .current_dir(&repo_path)
            .status()
            .unwrap();
        assert!(status.success());

        let tags = gather_env_tags(Source::ClaudeCode, Some(&repo_path), None);
        assert!(
            tags.contains(&"org:acme".to_string()),
            "expected org:acme in {tags:?}"
        );
    }

    #[test]
    fn no_org_tag_without_remote() {
        let repo = tempfile::TempDir::new().unwrap();
        let repo_path = repo.path().to_string_lossy().to_string();
        assert!(git_cmd(&["init"], Some(&repo_path)).is_some());

        let tags = gather_env_tags(Source::ClaudeCode, Some(&repo_path), None);
        assert!(!tags.iter().any(|t| t.starts_with("org:")));
    }
}