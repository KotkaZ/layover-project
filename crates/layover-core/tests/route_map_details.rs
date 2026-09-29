//! What the route map says about each agent: which model it runs on, and with how much context.
//!
//! Read from the command line Layover will run. A factory that fixes the model in its runner
//! command — one runner per model and permission set is a common shape — showed nothing at all,
//! because only `agent.model` was ever consulted.

use layover_core::Config;
use layover_core::diagram::{Layout, Live, Scope, render_svg};

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "shadow"

[runners.shadow]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh", "--context", "long_context", "--output-format", "json"]

[runners.claude]
command = ["claude", "-p", "--model", "{model}", "--output-format", "stream-json"]

[runners.plain]
command = ["copilot", "--allow-all-tools"]

[agents.analyst]
prompt = "analyse"
access = "read-only"

[agents.bob]
prompt = "build"
runner = "claude"
model = "claude-sonnet-5"

[agents.mailman]
prompt = "post"
runner = "plain"
access = "read-only"

[pipelines.devforge]
entry = "analyst"

[[routes]]
from = "analyst"
to = ["bob", "mailman"]
"#;

fn layout(text: &str) -> Layout {
    let config = Config::from_toml(text, "details.toml").expect("the fixture parses");
    Layout::scoped(
        &config,
        &Live::default(),
        &Scope::Pipeline("devforge".into()),
    )
}

#[test]
fn an_agent_shows_the_model_its_runner_command_fixes() {
    let layout = layout(FACTORY);
    let analyst = layout.node("a_analyst").expect("drawn");

    assert_eq!(analyst.subtitle.as_deref(), Some("claude-opus-5.5"));
}

#[test]
fn an_agent_shows_the_model_it_declares_when_its_runner_carries_it() {
    let layout = layout(FACTORY);
    let bob = layout.node("a_bob").expect("drawn");

    assert_eq!(bob.subtitle.as_deref(), Some("claude-sonnet-5"));
}

#[test]
fn the_context_tier_is_on_the_box_beside_the_access() {
    let svg = render_svg(&layout(FACTORY));

    assert!(svg.contains(">long context · read-only</text>"), "{svg}");
}

#[test]
fn the_tooltip_carries_what_the_box_has_no_room_for() {
    let svg = render_svg(&layout(FACTORY));
    let analyst = svg
        .split(r#"id="a_analyst""#)
        .nth(1)
        .and_then(|rest| rest.split("</g>").next())
        .expect("the analyst's node");

    for said in [
        "claude-opus-5.5",
        "reasoning xhigh",
        "long context",
        "runner shadow",
    ] {
        assert!(analyst.contains(said), "{said} missing from {analyst}");
    }
    assert!(analyst.contains("<title>"), "{analyst}");
}

#[test]
fn an_agent_whose_command_names_no_model_says_only_what_it_did_before() {
    let layout = layout(FACTORY);
    let mailman = layout.node("a_mailman").expect("drawn");

    assert_eq!(mailman.subtitle.as_deref(), Some("read-only"));
}

#[test]
fn a_factory_that_names_no_model_is_drawn_at_the_old_size() {
    let plain = FACTORY
        .replace(
            r#"command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh", "--context", "long_context", "--output-format", "json"]"#,
            r#"command = ["copilot"]"#,
        )
        .replace(r#"model = "claude-sonnet-5""#, "");
    let layout = layout(&plain);

    assert!(
        layout.nodes.iter().all(|node| (node.h - 56.0).abs() < 0.01),
        "{:?}",
        layout.nodes
    );
}
