//! A flag chosen when work is triggered reaches every run that work causes.
//!
//! The defect these pin: `POST /flights` with `{"flags":{"flag_x":true}}` was accepted and stored,
//! and every run was then composed from the pipeline's *defaults*, so the run received `off.md`
//! while `layover prompt --flag flag_x=true` showed `on.md`. The preview and the run disagreed, and
//! nothing said so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_tower::Factory;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-flags-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("prompts")).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ON: &str = "THE FLAG IS ON";
const OFF: &str = "THE FLAG IS OFF";

/// Three agents whose prompts all switch on `flag_x`: `a` is the entry, `b` is a hand-off and `c`
/// is reached over a spawn edge. `extra` is appended to the factory, for a second pipeline.
fn factory(temp: &Temp, extra: &str) -> Factory {
    let prompts = temp.0.join("prompts");
    for agent in ["a", "b", "c"] {
        std::fs::write(
            prompts.join(format!("{agent}.md")),
            "Do the work.\n@include(flag_x) on.md\n@include(!flag_x) off.md\n",
        )
        .expect("writes a prompt");
    }
    std::fs::write(prompts.join("on.md"), ON).expect("writes");
    std::fs::write(prompts.join("off.md"), OFF).expect("writes");

    let echo = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };

    let text = format!(
        r#"
[layover]
work_dir = "work"
prompt_dir = "prompts"

[defaults]
runner = "shell"
max_hops = 4
timeout_sec = 30

[runners.shell]
command = {echo}

[agents.a]
prompt_file = "a.md"
entry = true

[agents.b]
prompt_file = "b.md"

[agents.c]
prompt_file = "c.md"

[pipelines.p]
entry = "a"

[pipelines.p.flags]
flag_x = {{ default = false }}

[[routes]]
from = "a"
to = "b"

[[routes]]
from = "a"
to = "c"
mode = "spawn"
{extra}
"#
    );

    let config: Config = toml::from_str(&text).expect("the fixture factory parses");
    Factory::new(config, &temp.0).expect("opens")
}

fn flag_x(value: bool) -> BTreeMap<String, bool> {
    BTreeMap::from([("flag_x".to_owned(), value)])
}

fn queued(
    itinerary: &ItineraryId,
    from: Origin,
    to: &str,
    pipeline: Option<&str>,
    flags: BTreeMap<String, bool>,
) -> Queued {
    Queued::new(
        Flight::new(itinerary.clone(), from, AgentName::new(to), "hello", 4),
        pipeline.map(PipelineName::new),
        flags,
    )
}

/// The payload the one run of `agent` was given.
fn payload(root: &Path, agent: &str) -> String {
    std::fs::read_dir(root.join(".layover").join("hangars").join(agent))
        .unwrap_or_else(|_| panic!("`{agent}` never ran"))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("prompt.md"))
        .find(|path| path.exists())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_else(|| panic!("`{agent}` has no payload"))
}

fn assert_on(root: &Path, agent: &str) {
    let text = payload(root, agent);
    assert!(
        text.contains(ON) && !text.contains(OFF),
        "`{agent}` should have been composed with flag_x on:\n{text}"
    );
}

#[test]
fn a_flag_chosen_at_trigger_time_reaches_the_run() {
    let temp = Temp::new("trigger");
    let factory = factory(&temp, "");

    let drained = factory.drain(
        vec![queued(
            &ItineraryId::generate(),
            Origin::Human,
            "a",
            Some("p"),
            flag_x(true),
        )],
        |_| {},
        |_, _| {},
    );

    assert_eq!(drained.ran, 1);
    assert_on(&temp.0, "a");
}

#[test]
fn a_follow_on_flight_is_composed_with_its_chains_flags() {
    // Even a follow-on that carries nothing of its own — the shape `layover_send` queued before —
    // belongs to a chain, and the chain was triggered with the flag on.
    let temp = Temp::new("follow-on");
    let factory = factory(&temp, "");
    let chain = ItineraryId::generate();

    let mut handed_over = false;
    let drained = factory.drain_with(
        vec![queued(&chain, Origin::Human, "a", Some("p"), flag_x(true))],
        &mut |_| {},
        &mut |_, _| {},
        |_| {
            if handed_over {
                return Vec::new();
            }
            handed_over = true;
            vec![queued(
                &chain,
                Origin::Agent(AgentName::new("a")),
                "b",
                None,
                BTreeMap::new(),
            )]
        },
    );

    assert_eq!(drained.ran, 2);
    assert_on(&temp.0, "b");
}

#[test]
fn a_chains_flags_survive_a_restart() {
    // A fresh factory remembers nothing about the chain. The queued follow-on is all it has, so
    // that flight has to carry how the chain was triggered.
    let temp = Temp::new("restart");
    let factory = factory(&temp, "");

    factory.drain(
        vec![queued(
            &ItineraryId::generate(),
            Origin::Agent(AgentName::new("a")),
            "b",
            Some("p"),
            flag_x(true),
        )],
        |_| {},
        |_, _| {},
    );

    assert_on(&temp.0, "b");
}

#[test]
fn a_spawned_chain_is_composed_with_the_flags_it_inherited() {
    let temp = Temp::new("spawn");
    let factory = factory(&temp, "");

    factory.drain(
        vec![queued(
            &ItineraryId::generate(),
            Origin::Agent(AgentName::new("a")),
            "c",
            Some("p"),
            flag_x(true),
        )],
        |_| {},
        |_, _| {},
    );

    assert_on(&temp.0, "c");
}

#[test]
fn a_chain_no_pipeline_opened_is_composed_the_way_layover_prompt_previews_it() {
    // `layover prompt` without `--pipeline` offers every flag at the default of the first
    // pipeline to declare it. The run used to take the *last*, so when two pipelines disagreed the
    // preview and the run did too.
    let temp = Temp::new("no-pipeline");
    let factory = factory(
        &temp,
        r"
[pipelines.q]
entry = 'a'

[pipelines.q.flags]
flag_x = { default = true }
",
    );

    factory.drain(
        vec![queued(
            &ItineraryId::generate(),
            Origin::Human,
            "a",
            None,
            BTreeMap::new(),
        )],
        |_| {},
        |_, _| {},
    );

    let text = payload(&temp.0, "a");
    assert!(
        text.contains(OFF),
        "pipeline `p` declares flag_x first, with default false:\n{text}"
    );
}
