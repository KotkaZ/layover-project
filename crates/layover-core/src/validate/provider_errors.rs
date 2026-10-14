//! Checks about how a runner's output is read for a model-service failure.
//!
//! A declared matcher decides which failed runs are started again, so one that cannot match, or
//! matches anything, is worse than none: the first restarts nothing and says nothing, the second
//! restarts a run that failed its own work. Both are caught here, before a run depends on them.

use crate::config::Config;
use crate::provider::ProviderErrors;

use super::Diagnostic;

pub(super) fn check_provider_errors(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, runner) in &config.runners {
        if let Some(declared) = &runner.provider_errors {
            check(name, declared, found);
        }
    }
}

fn check(name: &str, declared: &ProviderErrors, found: &mut Vec<Diagnostic>) {
    let who = format!("runner `{name}`'s `provider_errors`");

    match (declared.read.is_some(), declared.failed.is_empty()) {
        (true, false) => found.push(Diagnostic::error(format!(
            "{who} sets both `read` and `failed`; give one. `read` uses a reader Layover has, \
             `failed` declares the events that say the model service failed"
        ))),
        (false, true) => found.push(Diagnostic::error(format!(
            "{who} sets neither `read` nor `failed`, so it cannot tell a model-service failure \
             from any other. Set `read = \"copilot\"`, `read = \"none\"`, or the `failed` events"
        ))),
        (true, true) if !declared.carried_on.is_empty() || declared.request_id.is_some() => {
            found.push(Diagnostic::error(format!(
                "{who} sets `carried_on` or `request_id` beside `read`. They describe declared \
                 `failed` events, which a reader Layover has does not use"
            )));
        }
        _ => {}
    }

    // A shape with no fields matches every event, so every failed run would be the service's.
    for (key, shapes) in [
        ("failed", &declared.failed),
        ("carried_on", &declared.carried_on),
    ] {
        if shapes.iter().any(serde_json::Map::is_empty) {
            found.push(Diagnostic::error(format!(
                "{who} has an empty table in `{key}`, which matches every event. Give each the \
                 fields that identify it, such as `{{ type = \"error\" }}`"
            )));
        }
    }

    if declared
        .request_id
        .as_deref()
        .is_some_and(|path| path.split('.').any(str::is_empty))
    {
        found.push(Diagnostic::error(format!(
            "{who} has a `request_id` path with an empty part; write it as dotted field names, \
             such as `error.request_id`"
        )));
    }

    if !declared.failed.is_empty() && declared.carried_on.is_empty() {
        found.push(Diagnostic::warning(format!(
            "{who} declares `failed` events and no `carried_on`, so a failure the CLI recovered \
             from by itself would still end the run as the service's, and a run that went on to \
             fail its own work would be started again. Declare the events that show the run \
             carried on: a tool starting, a call that succeeded"
        )));
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::validate::{Severity, validate};

    fn factory(provider_errors: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
[layover]
work_dir = "work"

[defaults]
runner = "main"

[runners.main]
command = ["my-cli", "--json"]
{provider_errors}

[agents.analyst]
description = "Analyses"
prompt = "analyse"
entry = true
"#
            ),
            "provider.toml",
        )
        .expect("parses")
    }

    fn said(config: &Config, severity: Severity) -> Vec<String> {
        validate(config)
            .into_iter()
            .filter(|d| d.severity == severity && d.message.contains("provider_errors"))
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn a_reader_or_a_complete_declaration_is_clean() {
        for clean in [
            "provider_errors = { read = \"copilot\" }",
            "provider_errors = { read = \"none\" }",
            r#"[runners.main.provider_errors]
failed = [{ type = "error", error.kind = "provider" }]
carried_on = [{ type = "tool_call" }]
request_id = "error.request_id""#,
        ] {
            let config = factory(clean);
            assert_eq!(
                said(&config, Severity::Error),
                Vec::<String>::new(),
                "{clean}"
            );
            assert_eq!(
                said(&config, Severity::Warning),
                Vec::<String>::new(),
                "{clean}"
            );
        }
    }

    #[test]
    fn a_declaration_that_cannot_decide_is_refused() {
        for (written, expected) in [
            (
                r#"provider_errors = { read = "copilot", failed = [{ type = "error" }] }"#,
                "sets both `read` and `failed`",
            ),
            (
                "provider_errors = { carried_on = [{ type = \"tool\" }] }",
                "neither",
            ),
            (
                r#"provider_errors = { read = "copilot", request_id = "id" }"#,
                "beside `read`",
            ),
            (
                "provider_errors = { failed = [{}], carried_on = [{ type = \"tool\" }] }",
                "matches every event",
            ),
            (
                r#"provider_errors = { failed = [{ type = "e" }], carried_on = [{ type = "t" }], request_id = "error..id" }"#,
                "empty part",
            ),
        ] {
            let errors = said(&factory(written), Severity::Error);
            assert!(
                errors.iter().any(|m| m.contains(expected)),
                "{written}: {errors:?}"
            );
        }
    }

    #[test]
    fn failures_with_no_sign_of_carrying_on_are_a_warning() {
        let config = factory(r#"provider_errors = { failed = [{ type = "error" }] }"#);

        assert!(
            said(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("no `carried_on`")),
            "{:?}",
            said(&config, Severity::Warning)
        );
    }
}
