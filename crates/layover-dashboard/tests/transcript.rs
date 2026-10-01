//! A transcript rendered the way the CLI would show it in a terminal.
//!
//! The Copilot events here are shaped like the ones Copilot CLI 1.0.88 writes with
//! `--output-format json`; the values are invented.

use layover_dashboard::transcript::{Entry, Kind, Renderer};

fn copilot(kind: &str, data: &str) -> String {
    format!(r#"{{"type":"{kind}","timestamp":"2026-10-01T08:00:00.000Z","data":{data}}}"#)
}

fn render(lines: &[String]) -> Vec<Entry> {
    let mut renderer = Renderer::new();
    for line in lines {
        renderer.line(line);
    }
    let (mut entries, _) = renderer.take();
    entries.extend(renderer.finish());
    entries
}

fn shown(entries: &[Entry]) -> Vec<(Kind, String)> {
    entries
        .iter()
        .map(|entry| (entry.kind, entry.text.clone()))
        .collect()
}

#[test]
fn a_copilot_session_reads_like_the_cli_would_show_it() {
    let prompt = "You are `eagle`.\n\nReview pull request 42.\nLook at the diff.\nThen report.";
    let lines = [
        copilot("session.mcp_servers_loaded", r#"{"servers":[{"name":"layover","status":"connected"}]}"#),
        copilot("user.message", &serde_json::json!({ "content": prompt, "delivery": "idle" }).to_string()),
        copilot("model.call_start", r#"{"model":"claude-opus-5.5","turnId":"0"}"#),
        copilot("assistant.reasoning_delta", r#"{"reasoningId":"r1","deltaContent":"Read the diff "}"#),
        copilot("assistant.reasoning_delta", r#"{"reasoningId":"r1","deltaContent":"first."}"#),
        copilot("assistant.reasoning", r#"{"reasoningId":"r1","content":"Read the diff first."}"#),
        copilot("assistant.tool_call_delta", r#"{"toolCallId":"c1","toolName":"view","inputDelta":"{\"pa"}"#),
        copilot(
            "tool.execution_start",
            r#"{"toolCallId":"c1","toolName":"view","arguments":{"path":"src/lib.rs","view_range":[1,40]}}"#,
        ),
        copilot(
            "tool.execution_complete",
            r#"{"toolCallId":"c1","success":true,"result":{"content":"1. //! The crate.\n2. pub mod a;","detailedContent":"…"}}"#,
        ),
        copilot(
            "tool.execution_start",
            r#"{"toolCallId":"c2","toolName":"powershell","arguments":{"command":"cargo test\ncargo clippy","description":"Test"}}"#,
        ),
        copilot(
            "tool.execution_complete",
            r#"{"toolCallId":"c2","success":false,"result":{}}"#,
        ),
        copilot("assistant.message", r#"{"messageId":"m1","content":"The change is sound.","toolRequests":[]}"#),
        copilot("session.usage_checkpoint", r#"{"totalNanoAiu":1510581560000,"totalPremiumRequests":15}"#),
        r#"{"type":"result","exitCode":0,"usage":{"premiumRequests":15,"sessionDurationMs":2806396}}"#.to_owned(),
    ];

    assert_eq!(
        shown(&render(&lines)),
        vec![
            (Kind::Info, "MCP servers: layover".to_owned()),
            (
                Kind::Prompt,
                "You are `eagle`.\nReview pull request 42.\n… 2 more line(s), 1 KB in all — \
                 prompt.md in the run's Hangar has every word"
                    .to_owned()
            ),
            (Kind::Info, "model claude-opus-5.5".to_owned()),
            (Kind::Think, "Read the diff first.".to_owned()),
            (Kind::Tool, "view src/lib.rs (1–40)".to_owned()),
            (Kind::Done, "1. //! The crate.\n2. pub mod a;".to_owned()),
            (Kind::Tool, "powershell $ cargo test …".to_owned()),
            (Kind::Failed, "powershell: failed".to_owned()),
            (Kind::Say, "The change is sound.".to_owned()),
            (Kind::Info, "1510.6 AI credits so far".to_owned()),
            (
                Kind::Info,
                "exited 0 · 15 premium request(s) · 46 min".to_owned()
            ),
        ],
        "deltas are not shown twice, and bookkeeping events not at all"
    );
}

#[test]
fn text_still_arriving_is_a_partial_that_the_finished_entry_replaces() {
    let mut renderer = Renderer::new();
    renderer.line(&copilot(
        "assistant.reasoning_delta",
        r#"{"reasoningId":"r9","deltaContent":"Weighing "}"#,
    ));
    let (entries, partials) = renderer.take();
    assert!(entries.is_empty());
    assert_eq!(partials.len(), 1);
    assert_eq!(
        (partials[0].kind, partials[0].text.as_str()),
        (Kind::Think, "Weighing ")
    );
    let id = partials[0].id.clone();

    renderer.line(&copilot(
        "assistant.reasoning_delta",
        r#"{"reasoningId":"r9","deltaContent":"the options."}"#,
    ));
    let (_, partials) = renderer.take();
    assert_eq!(
        partials[0].text, "Weighing the options.",
        "the whole block so far"
    );

    renderer.line(&copilot(
        "assistant.reasoning",
        r#"{"reasoningId":"r9","content":"Weighing the options."}"#,
    ));
    let (entries, partials) = renderer.take();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].closes.as_deref(), Some(id.as_str()));
    assert_eq!(partials.len(), 1);
    assert!(partials[0].text.is_empty(), "the partial is taken away");
}

#[test]
fn a_block_read_whole_in_one_go_never_shows_as_partial() {
    // Catching up on a run that has been going for an hour: every finished block arrives with its
    // deltas, and none of them should flash past as typing.
    let mut renderer = Renderer::new();
    renderer.line(&copilot(
        "assistant.message_delta",
        r#"{"messageId":"m1","deltaContent":"Done"}"#,
    ));
    renderer.line(&copilot(
        "assistant.message",
        r#"{"messageId":"m1","content":"Done."}"#,
    ));

    let (entries, partials) = renderer.take();
    assert!(partials.is_empty(), "{partials:?}");
    assert_eq!(entries[0].closes, None);
}

#[test]
fn a_block_that_never_finished_is_shown_as_far_as_it_got() {
    let entries = render(&[copilot(
        "assistant.message_delta",
        r#"{"messageId":"m2","deltaContent":"I was about to"}"#,
    )]);

    assert_eq!(
        shown(&entries),
        vec![(Kind::Say, "I was about to …".to_owned())]
    );
}

#[test]
fn a_long_tool_result_shows_its_first_lines_and_says_how_many_more() {
    let output = (1..=50)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let entries = render(&[
        copilot(
            "tool.execution_start",
            r#"{"toolCallId":"c","toolName":"grep","arguments":{"pattern":"fn main","paths":["src"]}}"#,
        ),
        copilot(
            "tool.execution_complete",
            &serde_json::json!({"toolCallId":"c","success":true,"result":{"content":output}})
                .to_string(),
        ),
    ]);

    assert_eq!(entries[0].text, r#"grep "fn main" in src"#);
    assert!(
        entries[1].text.starts_with("line 1\nline 2"),
        "{}",
        entries[1].text
    );
    assert!(
        entries[1].text.ends_with("line 8\n… 42 more line(s)"),
        "{}",
        entries[1].text
    );
}

#[test]
fn a_credential_in_the_output_is_masked() {
    let entries = render(&[copilot(
        "assistant.message",
        r#"{"messageId":"m","content":"Using ghp_0123456789abcdefABCDEF0123456789abcd to push."}"#,
    )]);

    assert!(!entries[0].text.contains("ghp_0123"), "{}", entries[0].text);
    assert!(
        entries[0].text.contains("[redacted]"),
        "{}",
        entries[0].text
    );
}

#[test]
fn a_claude_code_session_reads_the_same_way() {
    let lines = [
        r#"{"type":"system","subtype":"init","model":"claude-opus-4"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Check the tests."},{"type":"text","text":"Running them."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"1 test failed"}]}]}}"#,
        r#"{"type":"result","subtype":"success","total_cost_usd":0.42,"num_turns":3}"#,
    ]
    .map(str::to_owned);

    assert_eq!(
        shown(&render(&lines)),
        vec![
            (Kind::Info, "model claude-opus-4".to_owned()),
            (Kind::Think, "Check the tests.".to_owned()),
            (Kind::Say, "Running them.".to_owned()),
            (Kind::Tool, "Bash $ cargo test".to_owned()),
            (Kind::Failed, "Bash: 1 test failed".to_owned()),
            (Kind::Info, "success · $0.42 · 3 turn(s)".to_owned()),
        ]
    );
}

#[test]
fn output_in_no_known_dialect_is_shown_as_written_one_block_at_a_time() {
    let lines = [
        "Compiling layover v1.4.0",
        "error[E0308]: mismatched types",
        "",
        "  --> src/lib.rs:3:5",
    ]
    .map(str::to_owned);

    assert_eq!(
        shown(&render(&lines)),
        vec![(
            Kind::Raw,
            "Compiling layover v1.4.0\nerror[E0308]: mismatched types\n  --> src/lib.rs:3:5"
                .to_owned()
        )]
    );
}
