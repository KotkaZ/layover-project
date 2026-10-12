//! `[flags]`: declared once, accepted by every workflow, and composed into runs exactly as a
//! workflow's own flags are.

use layover_core::config::Config;
use layover_core::prompt::PromptMap;
use layover_core::validate::{Severity, validate, validate_prompts};

const FACTORY: &str = r#"
[runners.claude]
command = ["claude", "-p", "--output-format", "stream-json"]

[defaults]
runner = "claude"

[flags]
run_e2e = { default = false, description = "Also run the end-to-end suite" }

[agents.builder]
description = "Builds"
prompt_file = "builder.md"

[agents.tester]
description = "Tests"
prompt_file = "tester.md"

[pipelines.development]
entry = "builder"

[pipelines.nightly]
entry = "tester"
trigger = { every = "1d" }

[reserve]
fuel_usd = 100.0

[[routes]]
from = "builder"
to = "tester"
"#;

fn prompts() -> PromptMap {
    PromptMap::new()
        .with("builder.md", "Build it.\n")
        .with("tester.md", "Test it.\n@include(run_e2e) e2e.md\n")
        .with("e2e.md", "Run the end-to-end suite too.\n")
}

fn messages(config: &Config, severity: Severity) -> Vec<String> {
    validate(config)
        .into_iter()
        .chain(validate_prompts(config, &prompts()))
        .filter(|diagnostic| diagnostic.severity == severity)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn a_flag_declared_once_is_accepted_by_every_workflow_that_reaches_its_prompt() {
    let config = Config::from_toml(FACTORY, "flags.toml").expect("parses");

    assert_eq!(messages(&config, Severity::Error), Vec::<String>::new());
    for pipeline in ["development", "nightly"] {
        let flags = config.pipelines[&pipeline.into()]
            .flags_for_run(&[("run_e2e".to_owned(), true)].into())
            .expect("accepted");
        assert_eq!(flags.get("run_e2e"), Some(true), "{pipeline}");
    }
}

#[test]
fn without_it_each_workflow_would_have_to_declare_the_flag_itself() {
    let config = Config::from_toml(
        &FACTORY.replace(
            "[flags]\nrun_e2e = { default = false, description = \"Also run the end-to-end suite\" }\n",
            "",
        ),
        "flags.toml",
    )
    .expect("parses");

    let errors = messages(&config, Severity::Error);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors.iter().all(|error| error.contains("`run_e2e`")),
        "{errors:?}"
    );
}

#[test]
fn a_workflow_repeating_a_shared_flag_exactly_is_told_to_drop_its_copy() {
    let config = Config::from_toml(
        &format!(
            "{FACTORY}\n[pipelines.development.flags]\nrun_e2e = {{ default = false, description = \"Also run the end-to-end suite\" }}\n"
        ),
        "flags.toml",
    )
    .expect("parses");

    let warnings = messages(&config, Severity::Warning);
    assert!(
        warnings.iter().any(|warning| warning
            .contains("pipeline `development` declares flag `run_e2e` exactly as `[flags]` does")),
        "{warnings:?}"
    );
}

#[test]
fn a_workflow_may_still_declare_a_flag_of_its_own() {
    let config = Config::from_toml(
        &format!("{FACTORY}\n[pipelines.development.flags]\ndry_run = {{ default = true }}\n"),
        "flags.toml",
    )
    .expect("parses");

    let development = &config.pipelines[&"development".into()];
    assert!(development.flags.contains_key("dry_run"));
    assert!(development.flags.contains_key("run_e2e"));
    assert!(
        !config.pipelines[&"nightly".into()]
            .flags
            .contains_key("dry_run")
    );
}

#[test]
fn a_shared_flag_with_an_unusable_name_is_reported_once() {
    let config = Config::from_toml(&FACTORY.replace("run_e2e = {", "run-e2e = {"), "flags.toml")
        .expect("parses");

    let errors: Vec<String> = validate(&config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .filter(|message| message.contains("not a usable name"))
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("`[flags]`"), "{errors:?}");
}
