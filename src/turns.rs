use serde_json::Value;
use std::collections::HashMap;

use crate::transcript;

pub struct Turn {
    pub user_msg: Value,
    pub assistant_msgs: Vec<Value>,
    pub tool_results_by_id: HashMap<String, Value>,
}

pub fn build_turns(messages: Vec<Value>) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut current_user: Option<Value> = None;
    let mut assistant_order: Vec<String> = Vec::new();
    let mut assistant_latest: HashMap<String, Value> = HashMap::new();
    let mut tool_results_by_id: HashMap<String, Value> = HashMap::new();

    let flush = |current_user: &mut Option<Value>,
                 assistant_order: &mut Vec<String>,
                 assistant_latest: &mut HashMap<String, Value>,
                 tool_results_by_id: &mut HashMap<String, Value>,
                 turns: &mut Vec<Turn>| {
        if current_user.is_none() || assistant_latest.is_empty() {
            return;
        }
        let assistants: Vec<Value> = assistant_order
            .iter()
            .filter_map(|mid| assistant_latest.remove(mid))
            .collect();
        turns.push(Turn {
            user_msg: current_user.take().unwrap(),
            assistant_msgs: assistants,
            tool_results_by_id: std::mem::take(tool_results_by_id),
        });
        assistant_order.clear();
    };

    for msg in messages {
        // tool_result rows
        if transcript::is_tool_result(&msg) {
            for tr in transcript::iter_tool_results(transcript::get_content(&msg)) {
                if let Some(tid) = tr.get("tool_use_id").and_then(|v| v.as_str()) {
                    let content = tr.get("content").cloned().unwrap_or(Value::Null);
                    tool_results_by_id.insert(tid.to_string(), content);
                }
            }
            continue;
        }

        let role = transcript::get_role(&msg);

        if role == Some("user") {
            flush(
                &mut current_user,
                &mut assistant_order,
                &mut assistant_latest,
                &mut tool_results_by_id,
                &mut turns,
            );
            current_user = Some(msg);
            assistant_order.clear();
            assistant_latest.clear();
            tool_results_by_id.clear();
            continue;
        }

        if role == Some("assistant") {
            if current_user.is_none() {
                continue;
            }
            let mid = transcript::get_message_id(&msg)
                .map(String::from)
                .unwrap_or_else(|| format!("noid:{}", assistant_order.len()));
            if !assistant_latest.contains_key(&mid) {
                assistant_order.push(mid.clone());
            }
            assistant_latest.insert(mid, msg);
            continue;
        }
    }

    // flush last turn
    flush(
        &mut current_user,
        &mut assistant_order,
        &mut assistant_latest,
        &mut tool_results_by_id,
        &mut turns,
    );

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_single_turn() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"Hi"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_msgs.len(), 1);
    }

    #[test]
    fn builds_multiple_turns() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"First"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"Reply 1"}]}}),
            json!({"type":"user","message":{"role":"user","content":"Second"}}),
            json!({"type":"assistant","message":{"id":"m2","role":"assistant","model":"claude","content":[{"type":"text","text":"Reply 2"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 2);
    }

    #[test]
    fn collects_tool_results() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Do something"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[
                {"type":"text","text":"Let me check"},
                {"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}
            ]}}),
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"tu_1","content":"file1.txt\nfile2.txt"}
            ]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_results_by_id.contains_key("tu_1"));
    }

    #[test]
    fn dedupes_assistant_messages_by_id() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"partial"}]}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"full response"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_msgs.len(), 1);
        let text = transcript::extract_text(transcript::get_content(&turns[0].assistant_msgs[0]));
        assert_eq!(text, "full response");
    }
}
