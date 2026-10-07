use crate::config::Config;
use crate::validate::{Severity, validate};

/// A factory that is otherwise clean: every agent described, every runner printing its cost, so
/// the only warnings left are the ones these tests are about.
fn factory(runners: &str, agents: &str, defaults: &str) -> Config {
    Config::from_toml(
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shared"
{defaults}

{runners}

{agents}

[pipelines.review]
entry = "eagle"

[[routes]]
from = "eagle"
to = "tars"
"#
        ),
        "selection.toml",
    )
    .expect("parses")
}

const SHARED: &str = r#"
[runners.shared]
command = ["copilot", "--model", "{model}", "--reasoning-effort", "{effort}", "--context={context}", "--output-format", "json"]
"#;

const FIXED: &str = r#"
[runners.shared]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh", "--context", "long_context", "--output-format", "json"]
"#;

fn agents(eagle: &str, tars: &str) -> String {
    format!(
        r#"
[agents.eagle]
description = "Reviews"
prompt = "review"
model = "claude-opus-5.5"
{eagle}

[agents.tars]
description = "Triages"
prompt = "triage"
model = "claude-opus-5.5"
{tars}
"#
    )
}

fn warnings(config: &Config) -> Vec<String> {
    validate(config)
        .into_iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| d.message)
        .collect()
}

fn says(config: &Config, needle: &str) {
    let said = warnings(config);
    assert!(
        said.iter().any(|message| message.contains(needle)),
        "expected a warning containing {needle:?}, got {said:?}"
    );
}

#[test]
fn agents_using_the_placeholders_as_meant_are_silent() {
    let config = factory(
        SHARED,
        &agents(
            "effort = \"xhigh\"\ncontext = \"long_context\"",
            "effort = \"high\"\ncontext = \"long_context\"",
        ),
        "",
    );

    assert_eq!(warnings(&config), Vec::<String>::new());
}

#[test]
fn a_runner_that_fixes_everything_and_takes_no_placeholders_is_silent() {
    // Every factory written before the placeholders existed, and still a supported shape.
    let config = factory(
        FIXED,
        &agents("", "").replace("model = \"claude-opus-5.5\"\n", ""),
        "",
    );

    assert_eq!(warnings(&config), Vec::<String>::new());
}

#[test]
fn defaults_fill_in_what_agents_leave_unset_without_a_warning() {
    let config = factory(
        SHARED,
        &agents("effort = \"xhigh\"", ""),
        "effort = \"high\"\ncontext = \"long_context\"",
    );

    assert_eq!(warnings(&config), Vec::<String>::new());
}

#[test]
fn an_effort_or_context_the_runner_cannot_carry_is_a_warning() {
    let config = factory(
        FIXED,
        &agents("effort = \"low\"\ncontext = \"default\"", "")
            .replace("model = \"claude-opus-5.5\"\n", ""),
        "",
    );

    says(
        &config,
        "agent `eagle` sets `effort = \"low\"`, but runner `shared` has no `{effort}` placeholder",
    );
    says(
        &config,
        "agent `eagle` sets `context = \"default\"`, but runner `shared` has no `{context}` \
         placeholder",
    );
}

#[test]
fn a_placeholder_an_agent_leaves_unset_is_a_warning() {
    let config = factory(SHARED, &agents("effort = \"xhigh\"", ""), "");

    says(&config, "agent `tars` sets no `effort`");
    says(&config, "agent `tars` sets no `context`");
    says(&config, "agent `eagle` sets no `context`");
    assert!(
        warnings(&config)
            .iter()
            .all(|message| !message.contains("agent `eagle` sets no `effort`")),
        "eagle sets one: {:?}",
        warnings(&config)
    );
}

#[test]
fn an_empty_value_is_a_warning_rather_than_an_empty_argument() {
    let config = factory(SHARED, &agents("effort = \"\"", ""), "context = \" \"");

    says(
        &config,
        "agent `eagle` sets `effort = \"\"`, which counts as unset",
    );
    says(&config, "`[defaults] context` is empty");
}

#[test]
fn a_runner_that_fixes_a_value_beside_its_placeholder_is_a_warning() {
    let both = SHARED.replace(
        "\"--output-format\"",
        "\"--reasoning-effort\", \"high\", \"--output-format\"",
    );
    let config = factory(
        &both,
        &agents(
            "effort = \"xhigh\"\ncontext = \"default\"",
            "effort = \"low\"\ncontext = \"default\"",
        ),
        "",
    );

    says(
        &config,
        "runner `shared` passes `{effort}` and also fixes the same option to `high`",
    );
    assert_eq!(warnings(&config).len(), 1, "{:?}", warnings(&config));
}

#[test]
fn a_default_no_agent_can_pass_on_is_a_warning() {
    let config = factory(
        FIXED,
        &agents("", "").replace("model = \"claude-opus-5.5\"\n", ""),
        "effort = \"high\"",
    );

    says(&config, "`[defaults] effort = \"high\"` reaches no CLI");
}

#[test]
fn an_agent_with_no_model_on_a_model_placeholder_is_not_newly_warned_about() {
    // A joined `--model={model}` has always dropped out cleanly, and factories rely on it; a new
    // warning here would fail their `validate --strict`.
    let config = factory(
        r#"
[runners.shared]
command = ["codex", "exec", "--json", "--model={model}", "-"]
"#,
        &agents("", "").replace("model = \"claude-opus-5.5\"\n", ""),
        "",
    );

    assert!(
        warnings(&config)
            .iter()
            .all(|message| !message.contains("model")),
        "{:?}",
        warnings(&config)
    );
}

#[test]
fn a_model_the_runner_cannot_carry_is_still_a_warning() {
    let config = factory(FIXED, &agents("", ""), "");

    says(
        &config,
        "agent `eagle` sets `model = \"claude-opus-5.5\"`, but runner `shared` has no `{model}` \
         placeholder",
    );
}
