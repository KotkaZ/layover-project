//! Two agents on one runner, each run at its own effort and context, and the run records saying
//! which.
//!
//! The runner echoes its own arguments, so the transcript is the command line the CLI was handed —
//! which is the only proof that matters: a value that reached the record but not `argv` would be a
//! report of something that did not happen.

use layover_core::pipeline::PipelineName;
use layover_tower::Dispatched;

mod runs;
use runs::{Temp, factory, history, human};

/// A runner that prints its arguments, with the three placeholders in both of their forms.
fn echoes() -> String {
    let args = r#""--model", "{model}", "--reasoning-effort={effort}", "--context", "{context}", "--allow-all-tools""#;
    if cfg!(windows) {
        format!(r#"["cmd", "/c", "echo", {args}]"#)
    } else {
        format!(r#"["echo", {args}]"#)
    }
}

fn transcript_of(temp: &Temp, agent: &str) -> String {
    let hangar = temp.0.join(".layover").join("hangars").join(agent);
    std::fs::read_dir(&hangar)
        .expect("the agent ran")
        .filter_map(Result::ok)
        .filter_map(|run| std::fs::read_to_string(run.path().join("transcript.log")).ok())
        .collect()
}

#[test]
fn two_agents_on_one_runner_are_run_and_recorded_at_their_own_effort_and_context() {
    let temp = Temp::new("selection");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shared"
timeout_sec = 30
effort = "medium"

[runners.shared]
command = {}

[agents.eagle]
prompt = "review"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"
entry = true

[agents.tars]
prompt = "triage"
model = "claude-opus-5.5"
entry = true
"#,
            echoes()
        ),
    );

    let mut ran = 0;
    factory.drain(
        vec![human("eagle"), human("tars")],
        |_| {},
        |_, result| {
            if matches!(result, Dispatched::Ran { .. }) {
                ran += 1;
            }
        },
    );
    assert_eq!(ran, 2);

    let eagle = transcript_of(&temp, "eagle");
    assert!(
        eagle.contains("--model claude-opus-5.5 --reasoning-effort=xhigh --context long_context"),
        "{eagle}"
    );
    let tars = transcript_of(&temp, "tars");
    assert!(
        tars.contains("--model claude-opus-5.5 --reasoning-effort=medium --allow-all-tools"),
        "the default effort, and no `--context` left without its value: {tars}"
    );

    let runs = history(&temp.0);
    let record = |agent: &str| {
        runs.iter()
            .find(|run| run.agent.as_str() == agent)
            .map(|run| (run.model.clone(), run.effort.clone(), run.context.clone()))
            .expect("recorded")
    };
    assert_eq!(
        record("eagle"),
        (
            Some("claude-opus-5.5".to_owned()),
            Some("xhigh".to_owned()),
            Some("long_context".to_owned())
        )
    );
    assert_eq!(
        record("tars"),
        (
            Some("claude-opus-5.5".to_owned()),
            Some("medium".to_owned()),
            None
        )
    );
}

#[test]
fn a_workflow_runs_an_agent_at_its_own_override_and_says_so_in_the_record() {
    // The reviewer gates a feature at xhigh and sweeps pull requests at high: one agent, two
    // workflows, no second runner and no second agent.
    let temp = Temp::new("override");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shared"
timeout_sec = 30

[runners.shared]
command = {}

[agents.reviewer]
prompt = "review"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"
args = ["--deny-tool=shell(gh)"]

[pipelines.development]
entry = "reviewer"

[pipelines.sweep]
entry = "reviewer"

[pipelines.sweep.agents.reviewer]
effort = "high"
args = ["--deny-tool=shell(git)"]
"#,
            echoes()
        ),
    );

    let through = |pipeline: &str| {
        let mut queued = human("reviewer");
        queued.pipeline = Some(PipelineName::new(pipeline));
        queued
    };
    factory.drain(
        vec![through("development"), through("sweep")],
        |_| {},
        |_, _| {},
    );

    let runs = history(&temp.0);
    let effort_in = |pipeline: &str| {
        runs.iter()
            .find(|run| {
                run.pipeline
                    .as_ref()
                    .is_some_and(|p| p.as_str() == pipeline)
            })
            .and_then(|run| run.effort.clone())
    };
    assert_eq!(effort_in("development").as_deref(), Some("xhigh"));
    assert_eq!(effort_in("sweep").as_deref(), Some("high"));

    let lines = transcript_of(&temp, "reviewer");
    assert!(
        lines.contains("--reasoning-effort=high --context long_context --allow-all-tools --deny-tool=shell(gh) --deny-tool=shell(git)"),
        "the workflow's effort, and its args after the agent's own: {lines}"
    );
    assert!(
        lines.contains("--reasoning-effort=xhigh --context long_context --allow-all-tools --deny-tool=shell(gh)"),
        "{lines}"
    );
}

#[test]
fn a_named_chain_runs_with_its_choices_and_every_record_says_its_name() {
    let temp = Temp::new("chosen");
    let named = if cfg!(windows) {
        r#"["cmd", "/c", "echo", "--name={name}", "--reasoning-effort={effort}"]"#
    } else {
        r#"["echo", "--name={name}", "--reasoning-effort={effort}"]"#
    };
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "named"
timeout_sec = 30

[runners.named]
command = {named}

[agents.reviewer]
prompt = "review"
effort = "xhigh"
entry = true
"#
        ),
    );

    let mut queued = human("reviewer");
    queued.chosen = layover_core::chosen::Chosen {
        name: Some("Retry banner".to_owned()),
        agents: [(
            layover_core::agent::AgentName::new("reviewer"),
            layover_core::chosen::AgentChoice {
                effort: Some("max".to_owned()),
                ..layover_core::chosen::AgentChoice::default()
            },
        )]
        .into(),
    };
    factory.drain(vec![queued], |_| {}, |_, _| {});

    // `cmd /c echo` shows the quotes Windows puts round an argument with spaces in it.
    let line = transcript_of(&temp, "reviewer").replace('"', "");
    assert!(
        line.contains("--name=Retry banner - reviewer --reasoning-effort=max"),
        "{line}"
    );
    let runs = history(&temp.0);
    assert_eq!(runs[0].chain_name.as_deref(), Some("Retry banner"));
    assert_eq!(runs[0].effort.as_deref(), Some("max"));
}
