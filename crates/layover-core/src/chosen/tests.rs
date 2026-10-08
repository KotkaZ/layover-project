use super::*;

fn factory() -> Config {
    Config::from_toml(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
cli = "copilot"

[runners.script]
command = ["python", "agent.py"]

[agents.analyst]
prompt = "analyse"
model = "claude-opus-5.5"
effort = "xhigh"

[agents.reviewer]
prompt = "review"
model = "claude-opus-5.5"
effort = "xhigh"

[agents.notifier]
prompt = "notify"
runner = "script"

[agents.outsider]
prompt = "elsewhere"
entry = true

[pipelines.development]
entry = "analyst"

[pipelines.development.agents.reviewer]
effort = "high"

[[routes]]
from = "analyst"
to = ["reviewer", "notifier"]
"#,
        "chosen.toml",
    )
    .expect("parses")
}

fn choices(list: &[(&str, AgentChoice)]) -> BTreeMap<String, AgentChoice> {
    list.iter()
        .map(|(agent, choice)| ((*agent).to_owned(), choice.clone()))
        .collect()
}

fn effort(value: &str) -> AgentChoice {
    AgentChoice {
        effort: Some(value.to_owned()),
        ..AgentChoice::default()
    }
}

fn development() -> PipelineName {
    PipelineName::new("development")
}

#[test]
fn a_named_chain_with_a_choice_for_one_agent_is_kept_tidy() {
    let chosen = Chosen::checked(
        &factory(),
        Some(&development()),
        Some("  Login page: retry banner  "),
        &choices(&[
            ("reviewer", effort(" max ")),
            ("analyst", AgentChoice::default()),
        ]),
    )
    .expect("accepted");

    assert_eq!(chosen.name.as_deref(), Some("Login page: retry banner"));
    assert_eq!(
        chosen
            .agents
            .keys()
            .map(AgentName::as_str)
            .collect::<Vec<_>>(),
        ["reviewer"],
        "an empty choice is dropped"
    );
    assert_eq!(
        chosen.agents[&AgentName::from("reviewer")]
            .effort
            .as_deref(),
        Some("max")
    );
}

#[test]
fn what_a_choice_cannot_do_is_refused_with_the_reason() {
    let config = factory();
    let refused = |name: Option<&str>, agents: &[(&str, AgentChoice)]| {
        Chosen::checked(&config, Some(&development()), name, &choices(agents))
            .expect_err("refused")
            .0
    };

    assert!(refused(Some(&"x".repeat(NAME_LIMIT + 1)), &[]).contains("at most"));
    assert!(refused(Some("two\nlines"), &[]).contains("one line"));
    assert!(refused(None, &[("ghost", effort("high"))]).contains("not an agent"));
    assert!(refused(None, &[("outsider", effort("high"))]).contains("never runs in this workflow"));
    assert!(
        refused(None, &[("notifier", effort("high"))]).contains("no `{effort}` placeholder"),
        "a choice its CLI would never see"
    );
    assert!(
        refused(None, &[("reviewer", effort("high --allow-all"))]).contains("letters, digits"),
        "a value is a token, not a way to add arguments"
    );
}

#[test]
fn a_choice_wins_over_the_workflow_and_the_agent() {
    let config = factory();
    let chosen = Chosen {
        name: Some("Retry banner".to_owned()),
        agents: [(AgentName::from("reviewer"), effort("max"))].into(),
    };

    let reviewer = config.selection_in(&"reviewer".into(), Some(&development()), Some(&chosen));
    assert_eq!(reviewer.effort.as_deref(), Some("max"));
    assert_eq!(reviewer.name.as_deref(), Some("Retry banner - reviewer"));

    let analyst = config.selection_in(&"analyst".into(), Some(&development()), Some(&chosen));
    assert_eq!(
        analyst.effort.as_deref(),
        Some("xhigh"),
        "nothing chosen for it"
    );

    let unchosen = config.selection_in(&"reviewer".into(), Some(&development()), None);
    assert_eq!(
        unchosen.effort.as_deref(),
        Some("high"),
        "the workflow's override"
    );
    assert_eq!(unchosen.name, None);
}

#[test]
fn a_session_name_keeps_to_characters_a_command_line_passes_untouched() {
    let chosen = Chosen {
        name: Some(r#"Fix "retry" & <banner>; 50% off"#.to_owned()),
        agents: BTreeMap::new(),
    };

    assert_eq!(
        chosen.session_name(&"bob".into()).as_deref(),
        Some("Fix retry banner 50 off - bob")
    );
    assert_eq!(Chosen::default().session_name(&"bob".into()), None);
}

#[test]
fn what_was_chosen_survives_storage_and_nothing_chosen_writes_nothing() {
    let chosen = Chosen {
        name: Some("Retry banner".to_owned()),
        agents: [(AgentName::from("reviewer"), effort("max"))].into(),
    };
    let text = serde_json::to_string(&chosen).expect("serialises");
    assert_eq!(
        serde_json::from_str::<Chosen>(&text).expect("reads"),
        chosen
    );
    assert_eq!(
        serde_json::to_string(&Chosen::default()).expect("serialises"),
        "{}"
    );
}
