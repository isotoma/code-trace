//! Best-effort redaction of shape-recognizable secrets from trace content
//! before it is sent to Langfuse. High-precision patterns only (near-zero
//! false positives): it reduces leakage, it does not eliminate it. `pause` and
//! the git-repo gate remain the airtight privacy controls.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Whether secret masking is active. Defaults to `true`; set
/// `CODE_TRACE_MASK_SECRETS=false` to disable.
pub fn enabled() -> bool {
    match std::env::var("CODE_TRACE_MASK_SECRETS") {
        Ok(v) => v.trim().to_lowercase() != "false",
        Err(_) => true,
    }
}

struct Rule {
    re: Regex,
    /// Replacement string. May reference capture groups with `${1}` to preserve
    /// surrounding context (e.g. the `Bearer ` prefix or a URL's username).
    replacement: &'static str,
}

/// Ordered rules: specific before generic, so a placeholder emitted by an
/// earlier rule is never re-matched by a later one (e.g. `sk-ant-` before the
/// generic `sk-` OpenAI rule).
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |pattern: &str, replacement: &'static str| Rule {
            re: Regex::new(pattern).expect("secret pattern must compile"),
            replacement,
        };
        vec![
            // PEM private key blocks (any type), across newlines, non-greedy.
            r(
                r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
                "[REDACTED:private-key]",
            ),
            // AWS access key id.
            r(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "[REDACTED:aws-key]"),
            // GitHub tokens: classic (ghp_/gho_/ghu_/ghs_/ghr_) and fine-grained.
            r(
                r"\b(?:gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{22,})\b",
                "[REDACTED:github-token]",
            ),
            // Anthropic keys — must run before the generic OpenAI `sk-` rule.
            r(r"\bsk-ant-[A-Za-z0-9_-]{20,}", "[REDACTED:anthropic-key]"),
            // OpenAI-style keys.
            r(r"\bsk-[A-Za-z0-9]{32,}\b", "[REDACTED:openai-key]"),
            // Slack tokens.
            r(r"\bxox[baprs]-[A-Za-z0-9-]{10,}", "[REDACTED:slack-token]"),
            // Google API keys.
            r(r"\bAIza[0-9A-Za-z_-]{35}\b", "[REDACTED:google-key]"),
            // Stripe live keys.
            r(
                r"\b(?:sk|rk|pk)_live_[A-Za-z0-9]{16,}\b",
                "[REDACTED:stripe-key]",
            ),
            // JWTs (three base64url segments).
            r(
                r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
                "[REDACTED:jwt]",
            ),
            // Bearer auth header — keep the `Bearer ` prefix, drop the token.
            r(
                r"(?i)(Bearer\s+)[A-Za-z0-9._~+/=-]{10,}",
                "${1}[REDACTED:token]",
            ),
            // Credentials embedded in a URL — keep scheme+user, drop password.
            r(
                r"([a-zA-Z][a-zA-Z0-9+.-]*://[^\s/:@]+:)[^\s/:@]+@",
                "${1}[REDACTED:password]@",
            ),
        ]
    })
}

/// Redact secrets from `input`. Returns the redacted string and the number of
/// redactions applied.
pub fn mask_str(input: &str) -> (String, usize) {
    let mut text = input.to_string();
    let mut total = 0;
    for rule in rules() {
        let count = rule.re.find_iter(&text).count();
        if count > 0 {
            text = rule.re.replace_all(&text, rule.replacement).into_owned();
            total += count;
        }
    }
    (text, total)
}

/// Recurse over a JSON value, masking every string leaf in place. Returns the
/// total number of redactions across the whole value.
pub fn mask_value(v: &mut Value) -> usize {
    match v {
        Value::String(s) => {
            let (masked, n) = mask_str(s);
            if n > 0 {
                *s = masked;
            }
            n
        }
        Value::Array(arr) => arr.iter_mut().map(mask_value).sum(),
        Value::Object(map) => map.values_mut().map(mask_value).sum(),
        _ => 0,
    }
}

/// Serializes tests that read or mutate `CODE_TRACE_MASK_SECRETS` (a
/// process-global), so they don't observe each other's transient env state.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn masked(s: &str) -> String {
        mask_str(s).0
    }

    #[test]
    fn enabled_defaults_true_and_honors_false() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CODE_TRACE_MASK_SECRETS");
        assert!(enabled());
        std::env::set_var("CODE_TRACE_MASK_SECRETS", "false");
        assert!(!enabled());
        std::env::set_var("CODE_TRACE_MASK_SECRETS", "FALSE");
        assert!(!enabled());
        std::env::set_var("CODE_TRACE_MASK_SECRETS", "true");
        assert!(enabled());
        std::env::remove_var("CODE_TRACE_MASK_SECRETS");
    }

    #[test]
    fn redacts_aws_key_id() {
        let (out, n) = mask_str("key AKIAIOSFODNN7EXAMPLE here");
        assert_eq!(out, "key [REDACTED:aws-key] here");
        assert_eq!(n, 1);
    }

    #[test]
    fn redacts_github_tokens() {
        let t = format!("ghp_{}", "a".repeat(36));
        assert_eq!(masked(&t), "[REDACTED:github-token]");
        let pat = format!("github_pat_{}", "b".repeat(30));
        assert_eq!(masked(&pat), "[REDACTED:github-token]");
    }

    #[test]
    fn anthropic_runs_before_openai() {
        let t = format!("sk-ant-{}", "x".repeat(40));
        assert_eq!(masked(&t), "[REDACTED:anthropic-key]");
    }

    #[test]
    fn redacts_openai_key() {
        let t = format!("sk-{}", "A1b2".repeat(10)); // 40 chars after sk-
        assert_eq!(masked(&t), "[REDACTED:openai-key]");
    }

    #[test]
    fn redacts_slack_google_stripe() {
        assert_eq!(
            masked(&format!("xoxb-{}", "1234567890".repeat(2))),
            "[REDACTED:slack-token]"
        );
        assert_eq!(
            masked(&format!("AIza{}", "a".repeat(35))),
            "[REDACTED:google-key]"
        );
        assert_eq!(
            masked(&format!("sk_live_{}", "z".repeat(24))),
            "[REDACTED:stripe-key]"
        );
    }

    #[test]
    fn redacts_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcDEF-_123";
        assert_eq!(masked(jwt), "[REDACTED:jwt]");
    }

    #[test]
    fn redacts_private_key_block() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEabc\ndef123\n-----END RSA PRIVATE KEY-----";
        assert_eq!(masked(pem), "[REDACTED:private-key]");
    }

    #[test]
    fn redacts_bearer_but_keeps_prefix() {
        let (out, n) = mask_str("Authorization: Bearer abcdef123456ghijkl");
        assert_eq!(out, "Authorization: Bearer [REDACTED:token]");
        assert_eq!(n, 1);
    }

    #[test]
    fn redacts_url_password_but_keeps_user() {
        let (out, _) = mask_str("postgres://admin:supersecret@db.example.com:5432/app");
        assert_eq!(out, "postgres://admin:[REDACTED:password]@db.example.com:5432/app");
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let s = "just a normal sentence about ls and cargo build, no secrets";
        let (out, n) = mask_str(s);
        assert_eq!(out, s);
        assert_eq!(n, 0);
    }

    #[test]
    fn counts_multiple_redactions() {
        let s = format!("AKIAIOSFODNN7EXAMPLE and ghp_{}", "c".repeat(36));
        let (_, n) = mask_str(&s);
        assert_eq!(n, 2);
    }

    #[test]
    fn mask_value_recurses_into_nested_json() {
        let mut v = json!({
            "command": "deploy --token ghp_".to_string() + &"d".repeat(36),
            "nested": {"list": ["harmless", "AKIAIOSFODNN7EXAMPLE"]},
            "count": 42
        });
        let n = mask_value(&mut v);
        assert_eq!(n, 2);
        assert!(v["command"].as_str().unwrap().contains("[REDACTED:github-token]"));
        assert_eq!(v["nested"]["list"][1], "[REDACTED:aws-key]");
        assert_eq!(v["nested"]["list"][0], "harmless");
        assert_eq!(v["count"], 42); // non-strings untouched
    }
}
