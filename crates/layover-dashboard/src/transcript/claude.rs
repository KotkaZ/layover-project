//! Claude Code's `--output-format stream-json`: whole messages, one per line.
//!
//! `{"type":"assistant","message":{"content":[...]}}` carries what the model said, thought and
//! asked a tool to do; `{"type":"user",...}` carries what the tools gave back; `system` opens the
//! session and `result` closes it. Partial messages (`stream_event`) arrive only when asked for,
//! and are skipped: the whole message follows.

use serde_json::Value;

use super::{Kind, Renderer, summary, text_of};

/// Whether `event` is in Claude Code's dialect rather than Copilot's.
pub(super) fn speaks(event: &Value, kind: &str) -> bool {
    match kind {
        "assistant" | "user" => event.get("message").is_some(),
        "system" | "stream_event" => true,
        // Copilot's own `result` carries an exit code and premium requests; Claude's a subtype.
        "result" => event.get("subtype").is_some() || event.get("total_cost_usd").is_some(),
        _ => false,
    }
}

pub(super) fn event(out: &mut Renderer, event: &Value, kind: &str) {
    let blocks = event
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    match kind {
        "system" => {
            if let Some(model) = event.get("model").and_then(Value::as_str) {
                out.model(model, None);
            }
        }
        "assistant" => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => out.emit(Kind::Say, str_at(block, "text"), None, None),
                    Some("thinking") => {
                        out.emit(Kind::Think, str_at(block, "thinking"), None, None);
                    }
                    Some("tool_use") => {
                        let name = str_at(block, "name");
                        out.called(str_at(block, "id"), name);
                        let what = summary(block.get("input").unwrap_or(&Value::Null));
                        out.emit(Kind::Tool, format!("{name} {what}").trim_end(), None, None);
                    }
                    _ => {}
                }
            }
        }
        "user" => {
            if let Some(text) = event.pointer("/message/content").and_then(Value::as_str) {
                out.emit(Kind::Prompt, text, None, None);
            }
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let content = text_of(block.get("content").unwrap_or(&Value::Null));
                if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                    let name = out
                        .tool_named(str_at(block, "tool_use_id"))
                        .unwrap_or("tool")
                        .to_owned();
                    out.emit(Kind::Failed, &format!("{name}: {content}"), None, None);
                } else if content.trim().is_empty() {
                    out.emit(Kind::Done, "done", None, None);
                } else {
                    out.emit(Kind::Done, &content, None, None);
                }
            }
        }
        "result" => {
            let mut said = vec![str_at(event, "subtype").replace('_', " ")];
            if let Some(usd) = event.get("total_cost_usd").and_then(Value::as_f64) {
                said.push(format!("${usd:.2}"));
            }
            if let Some(turns) = event.get("num_turns").and_then(Value::as_u64) {
                said.push(format!("{turns} turn(s)"));
            }
            let failed = event.get("is_error").and_then(Value::as_bool) == Some(true);
            let kind = if failed { Kind::Failed } else { Kind::Info };
            out.emit(kind, &said.join(" · "), None, None);
        }
        _ => {}
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}
