//! How many runs may be alive at once: factory-wide, and per agent.

use layover_core::{Config, Severity, validate};

fn config(defaults: &str, agent: &str) -> Config {
    Config::from_toml(
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"
{defaults}

[runners.copilot]
command = ["copilot"]

[agents.mailman]
prompt = "post"
description = "posts"
entry = true
{agent}
"#
        ),
        "concurrency.toml",
    )
    .expect("parses")
}

fn errors(config: &Config) -> Vec<String> {
    validate(config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn an_agent_may_cap_its_own_overlap() {
    let config = config("", "max_concurrent = 1");

    assert_eq!(config.agents[&"mailman".into()].max_concurrent, Some(1));
    assert_eq!(config.defaults.max_concurrent_runs, 4, "the default stands");
    assert!(errors(&config).is_empty(), "{:?}", errors(&config));
}

#[test]
fn an_agent_left_unset_is_bound_only_by_the_factory() {
    assert_eq!(
        config("", "").agents[&"mailman".into()].max_concurrent,
        None
    );
}

#[test]
fn an_agent_capped_at_zero_would_never_run_and_is_an_error() {
    let agent = errors(&config("", "max_concurrent = 0"));
    assert!(
        agent
            .iter()
            .any(|message| message.contains("max_concurrent = 0")),
        "{agent:?}"
    );
}

#[test]
fn a_factory_limit_of_zero_still_loads_and_is_read_as_one() {
    // It loaded before this release, so refusing it now would break a working factory. It is said
    // out loud instead, because "zero at once" reads as "nothing" to anyone but the Tower.
    let config = config("max_concurrent_runs = 0", "");
    assert!(errors(&config).is_empty(), "{:?}", errors(&config));

    let warned: Vec<String> = validate(&config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .map(|diagnostic| diagnostic.message)
        .collect();
    assert!(
        warned.iter().any(|message| message.contains("treats as 1")),
        "{warned:?}"
    );
}
