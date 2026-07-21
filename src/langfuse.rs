use crate::emit::LangfuseConfig;
use crate::log;
use base64::Engine;
use serde_json::Value;

pub fn tracing_enabled() -> bool {
    std::env::var("TRACE_TO_LANGFUSE")
        .unwrap_or_default()
        .to_lowercase()
        == "true"
}

/// Whether tracing is restricted to sessions whose working directory is inside
/// a git repository. Defaults to `true`; set `CODE_TRACE_REQUIRE_GIT_REPO=false`
/// to trace everywhere.
pub fn require_git_repo() -> bool {
    match std::env::var("CODE_TRACE_REQUIRE_GIT_REPO") {
        Ok(v) => v.trim().to_lowercase() != "false",
        Err(_) => true,
    }
}

/// Resolve Langfuse credentials and host from the environment.
/// Returns None when either key is missing.
pub fn config_from_env() -> Option<LangfuseConfig> {
    let public_key = std::env::var("CC_LANGFUSE_PUBLIC_KEY")
        .or_else(|_| std::env::var("LANGFUSE_PUBLIC_KEY"))
        .unwrap_or_default();
    let secret_key = std::env::var("CC_LANGFUSE_SECRET_KEY")
        .or_else(|_| std::env::var("LANGFUSE_SECRET_KEY"))
        .unwrap_or_default();
    let host = std::env::var("CC_LANGFUSE_BASE_URL")
        .or_else(|_| std::env::var("LANGFUSE_BASE_URL"))
        .unwrap_or_else(|_| "https://cloud.langfuse.com".to_string());
    // Sanitize: stray quotes/whitespace/newlines or a trailing slash in the
    // base URL otherwise produce ureq's "invalid uri character" / redirect errors.
    let host = host
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim_end_matches('/')
        .to_string();

    if public_key.is_empty() || secret_key.is_empty() {
        return None;
    }
    Some(LangfuseConfig {
        host,
        public_key,
        secret_key,
    })
}

/// The Langfuse user id to attach to traces, if configured.
///
/// Optional: when unset (or empty) traces carry no `userId` and Langfuse's
/// user-scoped views simply won't group them. Typically an email address.
/// Accepts the `CC_LANGFUSE_` prefix like the other Langfuse variables.
pub fn user_id_from_env() -> Option<String> {
    let raw = std::env::var("CC_LANGFUSE_USER_ID")
        .or_else(|_| std::env::var("LANGFUSE_USER_ID"))
        .unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn auth_header(config: &LangfuseConfig) -> String {
    let credentials = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", config.public_key, config.secret_key));
    format!("Basic {credentials}")
}

pub fn get_json(config: &LangfuseConfig, path_and_query: &str) -> Result<Value, String> {
    let url = format!("{}{}", config.host, path_and_query);
    let resp = ureq::get(&url)
        .header("Authorization", &auth_header(config))
        .call()
        .map_err(|e| format!("GET {path_and_query} failed: {e}"))?;
    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| format!("GET {path_and_query}: reading body failed: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("GET {path_and_query}: invalid JSON: {e}"))
}

pub fn delete_json(config: &LangfuseConfig, path: &str, body: &Value) -> Result<Value, String> {
    let url = format!("{}{}", config.host, path);
    let resp = ureq::delete(&url)
        .header("Authorization", &auth_header(config))
        .force_send_body()
        .send_json(body)
        .map_err(|e| format!("DELETE {path} failed: {e}"))?;
    let text = resp
        .into_body()
        .read_to_string()
        .map_err(|e| format!("DELETE {path}: reading body failed: {e}"))?;
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("DELETE {path}: invalid JSON: {e}"))
}

/// Extract trace ids and total page count from a `GET /api/public/traces` page.
pub fn parse_traces_page(page: &Value) -> (Vec<String>, u64) {
    let ids = page
        .get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|t| t.get("id").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let total_pages = page
        .get("meta")
        .and_then(|m| m.get("totalPages"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    (ids, total_pages)
}

/// Collect all trace ids for a session, following pagination.
pub fn list_trace_ids(config: &LangfuseConfig, session_id: &str) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut page = 1u64;
    loop {
        let value = get_json(
            config,
            &format!("/api/public/traces?sessionId={session_id}&page={page}&limit=100"),
        )?;
        let (page_ids, total_pages) = parse_traces_page(&value);
        ids.extend(page_ids);
        if page >= total_pages {
            break;
        }
        page += 1;
    }
    Ok(ids)
}

/// The traces bulk-delete endpoint accepts at most 1000 ids per request.
pub const DELETE_CHUNK: usize = 1000;

/// Delete traces in chunks of `DELETE_CHUNK`. Returns how many ids were deleted.
/// Deleting a trace cascades to its child observations.
pub fn bulk_delete_traces(config: &LangfuseConfig, ids: &[String]) -> Result<usize, String> {
    for chunk in ids.chunks(DELETE_CHUNK) {
        let body = serde_json::json!({ "traceIds": chunk });
        delete_json(config, "/api/public/traces", &body)?;
        log::debug(&format!("deleted {} traces", chunk.len()));
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn require_git_repo_defaults_true_and_honors_false() {
        // Only this test touches CODE_TRACE_REQUIRE_GIT_REPO, so mutating the
        // process env here does not race other tests reading it.
        std::env::remove_var("CODE_TRACE_REQUIRE_GIT_REPO");
        assert!(require_git_repo(), "unset should default to true");

        std::env::set_var("CODE_TRACE_REQUIRE_GIT_REPO", "false");
        assert!(!require_git_repo(), "\"false\" disables the restriction");

        std::env::set_var("CODE_TRACE_REQUIRE_GIT_REPO", "FALSE");
        assert!(!require_git_repo(), "case-insensitive");

        std::env::set_var("CODE_TRACE_REQUIRE_GIT_REPO", "true");
        assert!(require_git_repo());

        std::env::set_var("CODE_TRACE_REQUIRE_GIT_REPO", "");
        assert!(require_git_repo(), "empty is not \"false\" -> restriction on");

        std::env::remove_var("CODE_TRACE_REQUIRE_GIT_REPO");
    }

    #[test]
    fn parses_page_ids_and_total() {
        let page = json!({
            "data": [{"id": "t1", "name": "x"}, {"id": "t2"}],
            "meta": {"page": 1, "limit": 100, "totalItems": 2, "totalPages": 1}
        });
        let (ids, total) = parse_traces_page(&page);
        assert_eq!(ids, vec!["t1", "t2"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn parses_empty_page() {
        let (ids, total) = parse_traces_page(&json!({"data": [], "meta": {"totalPages": 0}}));
        assert!(ids.is_empty());
        assert_eq!(total, 0);
        let (ids, total) = parse_traces_page(&json!({}));
        assert!(ids.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn chunking_splits_at_1000() {
        let ids: Vec<String> = (0..2500).map(|i| format!("t{i}")).collect();
        let chunks: Vec<_> = ids.chunks(DELETE_CHUNK).collect();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1000);
        assert_eq!(chunks[1].len(), 1000);
        assert_eq!(chunks[2].len(), 500);
    }
}
