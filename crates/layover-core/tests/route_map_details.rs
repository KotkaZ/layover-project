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
fn an_agent_shows_the_model_and_effort_its_runner_command_fixes() {
    // Effort sits beside the model because it is how hard that model is run: the two are one
    // choice, and reading one without the other says half of it.
    let layout = layout(FACTORY);
    let analyst = layout.node("a_analyst").expect("drawn");

    assert_eq!(
        analyst.subtitle.as_deref(),
        Some("claude-opus-5.5 · effort xhigh")
    );
    assert_eq!(analyst.caption.as_deref(), Some("long context · read-only"));
}

#[test]
fn an_agent_whose_command_sets_effort_but_no_model_still_shows_the_effort() {
    let text = FACTORY.replace(
        r#"command = ["copilot", "--allow-all-tools"]"#,
        r#"command = ["copilot", "--allow-all-tools", "--reasoning-effort", "high"]"#,
    );
    let layout = layout(&text);
    let mailman = layout.node("a_mailman").expect("drawn");

    assert_eq!(mailman.subtitle.as_deref(), Some("effort high · read-only"));
    assert_eq!(mailman.caption, None);
}

#[test]
fn a_model_name_too_long_to_share_its_line_moves_the_effort_to_the_next() {
    // Cutting the line short would drop the effort, which is the part that was asked for; and
    // what no longer fits beside it gets a line of its own rather than being cut off either.
    let text = FACTORY.replace("claude-opus-5.5", "claude-opus-5.5-preview");
    let layout = layout(&text);
    let analyst = layout.node("a_analyst").expect("drawn");

    assert_eq!(analyst.subtitle.as_deref(), Some("claude-opus-5.5-preview"));
    assert_eq!(
        analyst.caption.as_deref(),
        Some("effort xhigh · long context")
    );
    assert_eq!(analyst.footnote.as_deref(), Some("read-only"));
    assert!(
        layout.nodes.iter().all(|node| (node.h - 84.0).abs() < 0.01),
        "every box grows together, so the rows still line up"
    );
}

#[test]
fn no_line_in_a_box_is_longer_than_a_box_is_wide() {
    // Measured in the dashboard's own font: thirty characters is about 130 of the 148 pixels a
    // box has between its padding.
    for text in [
        FACTORY.to_owned(),
        FACTORY.replace("claude-opus-5.5", "claude-opus-5.5-preview"),
    ] {
        for node in &layout(&text).nodes {
            for line in [&node.subtitle, &node.caption, &node.footnote]
                .into_iter()
                .flatten()
            {
                assert!(line.chars().count() <= 30, "{line:?} in {}", node.id);
            }
        }
    }
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
        "effort xhigh",
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
