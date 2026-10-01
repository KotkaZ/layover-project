//! Copilot CLI's `--output-format json`: one event per line, `{"type": "...", "data": {...}}`.
//!
//! Shapes read from real transcripts written by Copilot CLI 1.0.88. Events not named here — turn
//! boundaries, cache telemetry, the deltas of a tool's arguments — are bookkeeping, and skipped.

use std::fmt::Write as _;

use serde_json::Value;

use super::{Kind, Renderer, first_line, summary, text_of};

pub(super) fn event(out: &mut Renderer, event: &Value, kind: &str) {
    let data = event.get("data").unwrap_or(&Value::Null);
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or_default();
    let at = event.get("timestamp").and_then(Value::as_str);

    match kind {
        "user.message" => prompt(out, text("content"), at),
        "assistant.reasoning_delta" => {
            out.grow(
                format!("r:{}", text("reasoningId")),
                Kind::Think,
                text("deltaContent"),
            );
        }
        "assistant.reasoning" => {
            let closes = out.close(format!("r:{}", text("reasoningId")));
            out.emit(Kind::Think, text("content"), at, closes);
        }
        "assistant.message_delta" => {
            out.grow(
                format!("m:{}", text("messageId")),
                Kind::Say,
                text("deltaContent"),
            );
        }
        "assistant.message" => {
            let closes = out.close(format!("m:{}", text("messageId")));
            out.emit(Kind::Say, text("content"), at, closes);
        }
        "tool.execution_start" => {
            let name = text("toolName");
            out.called(text("toolCallId"), name);
            let what = summary(data.get("arguments").unwrap_or(&Value::Null));
            out.emit(Kind::Tool, format!("{name} {what}").trim_end(), at, None);
        }
        "tool.execution_partial_result" => {
            out.set(
                format!("t:{}", text("toolCallId")),
                Kind::Done,
                text("partialOutput"),
            );
        }
        "tool.execution_complete" => complete(out, data, at),
        "result" => result(out, event, at),
        other if other.contains("error") => {
            let message = data
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| data.to_string(), str::to_owned);
            out.emit(Kind::Failed, &format!("{other}: {message}"), at, None);
        }
        other => session(out, other, data, at),
    }
}

/// What the CLI says about the session around the work: subagents, servers, the model, the bill.
fn session(out: &mut Renderer, kind: &str, data: &Value, at: Option<&str>) {
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or_default();

    match kind {
        "subagent.started" => out.emit(
            Kind::Info,
            &format!(
                "subagent {} started on {}",
                text("agentDisplayName"),
                text("model")
            ),
            at,
            None,
        ),
        "subagent.completed" => {
            let seconds = data.get("durationMs").and_then(Value::as_u64).unwrap_or(0) / 1_000;
            let calls = data
                .get("totalToolCalls")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            out.emit(
                Kind::Info,
                &format!(
                    "subagent {} finished in {seconds}s after {calls} tool call(s)",
                    text("agentDisplayName")
                ),
                at,
                None,
            );
        }
        "model.call_start" if !text("model").is_empty() => out.model(text("model"), at),
        "session.info" => out.emit(Kind::Info, text("message"), at, None),
        "session.mcp_servers_loaded" => {
            let names: Vec<&str> = data
                .get("servers")
                .and_then(Value::as_array)
                .map(|servers| {
                    servers
                        .iter()
                        .filter_map(|server| {
                            server
                                .get("name")
                                .and_then(Value::as_str)
                                .or(server.as_str())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !names.is_empty() {
                out.emit(
                    Kind::Info,
                    &format!("MCP servers: {}", names.join(", ")),
                    at,
                    None,
                );
            }
        }
        "session.mcp_server_status_changed" => out.emit(
            Kind::Info,
            &format!("MCP server {}: {}", text("serverName"), text("status")),
            at,
            None,
        ),
        "session.usage_checkpoint" => {
            #[expect(clippy::cast_precision_loss, reason = "a figure for a person to read")]
            let credits = data
                .get("totalNanoAiu")
                .and_then(Value::as_u64)
                .unwrap_or(0) as f64
                / 1e9;
            out.emit(
                Kind::Info,
                &format!("{credits:.1} AI credits so far"),
                at,
                None,
            );
        }
        _ => {}
    }
}

/// What the run was asked, as its first few lines: the whole prompt is in the run's Hangar.
fn prompt(out: &mut Renderer, content: &str, at: Option<&str>) {
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let mut shown = lines.iter().take(2).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > 2 {
        let _ = write!(
            shown,
            "\n… {} more line(s), {} KB in all — prompt.md in the run's Hangar has every word",
            lines.len() - 2,
            content.len().div_ceil(1_024)
        );
    }
    out.emit(Kind::Prompt, &shown, at, None);
}

/// What a tool gave back, or why it failed.
fn complete(out: &mut Renderer, data: &Value, at: Option<&str>) {
    let id = data
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let closes = out.close(format!("t:{id}"));
    let name = out.tool_named(id).unwrap_or("tool").to_owned();
    let result = data.get("result").unwrap_or(&Value::Null);
    let content = text_of(result.get("content").unwrap_or(&Value::Null));

    if data.get("success").and_then(Value::as_bool) == Some(false) {
        let why = data
            .get("error")
            .map(|error| error.get("message").map_or_else(|| text_of(error), text_of))
            .filter(|why| !why.trim().is_empty())
            .unwrap_or(content);
        let why = if why.trim().is_empty() {
            "failed".to_owned()
        } else {
            why
        };
        out.emit(Kind::Failed, &format!("{name}: {why}"), at, closes);
    } else if content.trim().is_empty() {
        out.emit(Kind::Done, "done", at, closes);
    } else {
        out.emit(Kind::Done, &content, at, closes);
    }
}

/// How the session ended, as the CLI reported it.
fn result(out: &mut Renderer, event: &Value, at: Option<&str>) {
    let mut said = Vec::new();
    if let Some(code) = event.get("exitCode").and_then(Value::as_i64) {
        said.push(format!("exited {code}"));
    }
    let usage = event.get("usage").unwrap_or(&Value::Null);
    if let Some(premium) = usage.get("premiumRequests").and_then(Value::as_u64) {
        said.push(format!("{premium} premium request(s)"));
    }
    if let Some(ms) = usage.get("sessionDurationMs").and_then(Value::as_u64) {
        said.push(format!("{} min", ms / 60_000));
    }
    let failed = event
        .get("exitCode")
        .and_then(Value::as_i64)
        .is_some_and(|code| code != 0);
    let kind = if failed { Kind::Failed } else { Kind::Info };
    out.emit(kind, &first_line(&said.join(" · ")), at, None);
}
