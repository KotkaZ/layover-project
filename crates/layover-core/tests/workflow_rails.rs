//! Validation measures each workflow against its own rails.

use layover_core::config::Config;
use layover_core::validate::{Severity, validate};

fn messages(text: &str, severity: Severity) -> Vec<String> {
    let config = Config::from_toml(text, "rails.toml").expect("parses");
    validate(&config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// A line of four agents, entered at the first, with `[defaults] max_hops = 2`.
fn line(rails: &str) -> String {
    format!(
        r#"
[defaults]
runner = "claude"
max_hops = 2

[runners.claude]
cli = "claude"

[agents.a]
description = "a"
prompt = "a"

[agents.b]
description = "b"
prompt = "b"

[agents.c]
description = "c"
prompt = "c"

[agents.d]
description = "d"
prompt = "d"

[pipelines.long]
entry = "a"
{rails}

[[routes]]
from = "a"
to = "b"

[[routes]]
from = "b"
to = "c"

[[routes]]
from = "c"
to = "d"
"#
    )
}

#[test]
fn an_agent_too_deep_for_the_factory_is_within_reach_of_a_workflow_given_the_hops() {
    let short = messages(&line(""), Severity::Warning);
    assert!(
        short
            .iter()
            .any(|warning| warning.contains("agent `d` is 4 flights")),
        "{short:?}"
    );

    let long = messages(&line("max_hops = 6"), Severity::Warning);
    assert!(
        long.iter()
            .all(|warning| !warning.contains("flights from the nearest")),
        "{long:?}"
    );
}

#[test]
fn a_workflow_rail_of_zero_is_refused() {
    for (rails, says) in [
        ("fuel_usd = 0.0", "sets `fuel_usd = 0`"),
        ("max_runs = 0", "sets `max_runs` to 0"),
        ("max_hops = 0", "sets `max_hops` to 0"),
        ("timeout = \"0s\"", "sets `timeout` to 0"),
    ] {
        let errors = messages(&line(rails), Severity::Error);
        assert!(
            errors.iter().any(|error| error.contains(says)),
            "{rails}: {errors:?}"
        );
    }
}
