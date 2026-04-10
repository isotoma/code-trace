use std::process::Command;

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

pub fn gather_env_tags(cwd: Option<&str>) -> Vec<String> {
    let mut tags = vec!["claude-code".to_string()];

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

    // Claude Code version
    if let Ok(output) = Command::new("claude").arg("--version").output() {
        if output.status.success() {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !ver.is_empty() {
                tags.push(format!("cc-version:{ver}"));
            }
        }
    }

    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_claude_code_tag() {
        let tags = gather_env_tags(None);
        assert!(tags.contains(&"claude-code".to_string()));
    }

    #[test]
    fn includes_os_tag() {
        let tags = gather_env_tags(None);
        assert!(tags.iter().any(|t| t.starts_with("os:")));
    }

    #[test]
    fn includes_user_tag() {
        let tags = gather_env_tags(None);
        assert!(tags.iter().any(|t| t.starts_with("user:")));
    }
}
