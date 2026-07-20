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

pub fn parse_config_str(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = strip_quotes(value.trim());
            if !key.is_empty() {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Strip one layer of matching surrounding quotes (single or double).
/// Users often quote config values out of habit (e.g. `LANGFUSE_BASE_URL="https://..."`);
/// without this the quotes end up inside the value and break the Langfuse URL.
fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return &value[1..value.len() - 1];
        }
    }
    value
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

    #[test]
    fn trims_whitespace_around_key_and_value() {
        let content = "TRACE_TO_LANGFUSE = true \n";
        let map = parse_config_str(content);
        assert_eq!(map.get("TRACE_TO_LANGFUSE"), Some(&"true".to_string()));
    }

    #[test]
    fn strips_surrounding_double_quotes() {
        let content = "LANGFUSE_BASE_URL=\"https://cloud.langfuse.com\"\n";
        let map = parse_config_str(content);
        assert_eq!(
            map.get("LANGFUSE_BASE_URL"),
            Some(&"https://cloud.langfuse.com".to_string())
        );
    }

    #[test]
    fn strips_surrounding_single_quotes() {
        let content = "LANGFUSE_BASE_URL='https://cloud.langfuse.com'\n";
        let map = parse_config_str(content);
        assert_eq!(
            map.get("LANGFUSE_BASE_URL"),
            Some(&"https://cloud.langfuse.com".to_string())
        );
    }

    #[test]
    fn keeps_interior_and_unmatched_quotes() {
        // Unmatched leading quote is left untouched.
        let map = parse_config_str("K=\"value\n");
        assert_eq!(map.get("K"), Some(&"\"value".to_string()));
        // A lone quote char stays as-is (len < 2 guard).
        let map = parse_config_str("K=\"\n");
        assert_eq!(map.get("K"), Some(&"\"".to_string()));
    }

    #[test]
    fn skips_lines_with_empty_key() {
        let content = "=orphan_value\nTRACE_TO_LANGFUSE=true\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key(""));
    }
}
