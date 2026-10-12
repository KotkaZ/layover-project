//! `routes_from`: a follow-up uses the scoped routes of the workflow whose work it resumes, without
//! every one of those routes having to name it.

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::scope::RouteMap;
use layover_core::validate::{Severity, validate};

const FACTORY: &str = r#"
[defaults]
runner = "claude"

[runners.claude]
cli = "claude"

[agents.analyst]
prompt = "a"

[agents.builder]
prompt = "b"

[agents.reviewer]
prompt = "r"

[pipelines.development]
entry = "analyst"

[pipelines.follow-up]
trigger = { every = "30m" }
resumes = true
routes_from = "development"

[pipelines.sweep]
entry = "reviewer"

[[routes]]
from = "analyst"
to = "builder"
pipelines = "development"

[[routes]]
from = "reviewer"
to = "builder"
pipelines = "sweep"
"#;

fn errors(config: &Config) -> Vec<String> {
    validate(config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn a_follow_up_may_use_the_routes_of_the_workflow_it_takes_them_from_and_no_others() {
    let config = Config::from_toml(FACTORY, "routes_from.toml").expect("parses");
    let routes = RouteMap::from_config(&config);
    let (analyst, builder, reviewer) = (
        AgentName::from("analyst"),
        AgentName::from("builder"),
        AgentName::from("reviewer"),
    );

    let follow_up = routes.for_pipeline(Some(&"follow-up".into()));
    assert!(follow_up.permits(&analyst, &builder), "development's route");
    assert!(!follow_up.permits(&reviewer, &builder), "not the sweep's");

    let sweep = routes.for_pipeline(Some(&"sweep".into()));
    assert!(
        !sweep.permits(&analyst, &builder),
        "development's route stays scoped"
    );
    assert!(errors(&config).is_empty(), "{:?}", errors(&config));
}

#[test]
fn routes_taken_from_a_workflow_that_takes_them_from_another_carry_through() {
    let config = Config::from_toml(
        &format!(
            "{FACTORY}\n[pipelines.second-follow-up]\ntrigger = {{ every = \"1h\" }}\nresumes = true\nroutes_from = \"follow-up\"\n"
        ),
        "routes_from.toml",
    )
    .expect("parses");

    assert!(
        RouteMap::from_config(&config)
            .for_pipeline(Some(&"second-follow-up".into()))
            .permits(&"analyst".into(), &"builder".into())
    );
}

#[test]
fn naming_itself_or_a_pipeline_that_does_not_exist_is_refused() {
    for (from, says) in [
        ("follow-up", "takes its routes from itself"),
        (
            "nowhere",
            "takes its routes from `nowhere`, which is not a pipeline",
        ),
    ] {
        let config = Config::from_toml(
            &FACTORY.replace(
                "routes_from = \"development\"",
                &format!("routes_from = \"{from}\""),
            ),
            "routes_from.toml",
        )
        .expect("parses");

        let errors = errors(&config);
        assert!(
            errors.iter().any(|error| error.contains(says)),
            "{errors:?}"
        );
    }
}
