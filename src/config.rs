use std::collections::HashMap;

pub fn load_config() -> HashMap<String, String> {
    let path = config_path();
    parse_config_file(&path)
}

fn config_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            std::path::PathBuf::from(home).join(".config")
        });
    base.join("code-trace").join("config")
}

fn parse_config_file(path: &std::path::Path) -> HashMap<String, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_config_str(&content)
}

fn parse_config_str(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_pairs() {
        let content = "TRACE_TO_LANGFUSE=true\nLANGFUSE_PUBLIC_KEY=pk-lf-abc\n";
        let map = parse_config_str(content);
        assert_eq!(map.get("TRACE_TO_LANGFUSE"), Some(&"true".to_string()));
        assert_eq!(map.get("LANGFUSE_PUBLIC_KEY"), Some(&"pk-lf-abc".to_string()));
    }

    #[test]
    fn skips_comment_lines() {
        let content = "# this is a comment\nTRACE_TO_LANGFUSE=true\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("TRACE_TO_LANGFUSE"), Some(&"true".to_string()));
    }

    #[test]
    fn skips_blank_lines() {
        let content = "\nTRACE_TO_LANGFUSE=true\n\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn skips_lines_without_equals() {
        let content = "INVALID_LINE\nTRACE_TO_LANGFUSE=true\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn value_can_contain_equals() {
        let content = "LANGFUSE_SECRET_KEY=sk-lf-a=b=c\n";
        let map = parse_config_str(content);
        assert_eq!(map.get("LANGFUSE_SECRET_KEY"), Some(&"sk-lf-a=b=c".to_string()));
    }

    #[test]
    fn missing_file_returns_empty_map() {
        let map = parse_config_file(std::path::Path::new("/tmp/code-trace-does-not-exist.cfg"));
        assert!(map.is_empty());
    }
}
