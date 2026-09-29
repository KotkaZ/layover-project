//! Pipeline-scoped routes on the maps: each workflow's diagram draws exactly what its own routes
//! reach, and the whole-factory map says which workflows each scoped edge belongs to.

mod common;

use common::scoped::{agents, config, karls_factory};
use layover_core::Config;
use layover_core::diagram::{Layout, Live, Scope};

/// The agents a workflow's diagram draws.
fn drawn(config: &Config, pipeline: &str) -> Vec<String> {
    let layout = Layout::scoped(config, &Live::default(), &Scope::Pipeline(pipeline.into()));
    let mut names: Vec<String> = layout
        .nodes
        .iter()
        .filter_map(|node| node.id.strip_prefix("a_").map(str::to_owned))
        .collect();
    names.sort();
    names
}

#[test]
fn each_workflows_map_draws_exactly_what_its_routes_reach() {
    let config = karls_factory();

    assert_eq!(
        drawn(&config, "eagle-eye"),
        ["azurix", "eagle", "golddigger", "sherlock"]
    );
    assert_eq!(
        drawn(&config, "chromium-webrtc-monitor"),
        ["mailman", "tars"]
    );
    assert_eq!(
        drawn(&config, "devforge"),
        [
            "analyst",
            "azurix",
            "bob",
            "eagle",
            "golddigger",
            "mailman",
            "sherlock",
            "wolf"
        ]
    );
}

#[test]
fn a_workflows_map_draws_only_its_own_edges() {
    let config = karls_factory();
    let eagle_eye = Layout::scoped(
        &config,
        &Live::default(),
        &Scope::Pipeline("eagle-eye".into()),
    );

    assert!(
        eagle_eye
            .edges
            .iter()
            .all(|edge| edge.to != "a_bob" && edge.from != "a_bob"),
        "Eagle Eye never reaches bob"
    );
    assert!(
        !eagle_eye
            .edges
            .iter()
            .any(|edge| edge.from == "a_sherlock" && edge.to == "a_analyst"),
        "a DevForge-only edge between two shared agents is not Eagle Eye's"
    );
}

#[test]
fn the_whole_factory_map_marks_which_edges_belong_to_which_workflows() {
    let config = karls_factory();
    let everything = Layout::build(&config, &Live::default());

    let spawn = everything
        .edges
        .iter()
        .find(|edge| edge.from == "a_azurix" && edge.to == "a_eagle")
        .expect("the spawn edge is drawn");
    assert_eq!(spawn.scopes, ["eagle-eye"]);

    let shared = everything
        .edges
        .iter()
        .find(|edge| edge.from == "a_eagle" && edge.to == "a_sherlock")
        .expect("drawn once");
    assert_eq!(
        shared.scopes,
        ["devforge", "devforge-follow-up", "eagle-eye"],
        "one edge, every scope that permits it"
    );
}

#[test]
fn mermaid_marks_a_scoped_edge_and_leaves_a_global_one_as_it_was() {
    let scoped = layover_core::route_map(&karls_factory(), &Live::default());
    assert!(
        scoped.contains(r#"a_azurix -->|"spawn · eagle-eye"| a_eagle"#),
        "{scoped}"
    );
    assert!(
        scoped.contains(r#"a_tars -->|"chromium-webrtc-monitor"| a_mailman"#),
        "{scoped}"
    );

    let plain = config(&format!(
        "{}\n[pipelines.go]\nentry = \"a\"\n\n[[routes]]\nfrom = \"a\"\nto = \"b\"\n",
        agents(&["a", "b"])
    ));
    assert!(
        layover_core::route_map(&plain, &Live::default()).contains("a_a --> a_b\n"),
        "an unscoped factory's diagram is unchanged"
    );
}

#[test]
fn mermaid_can_draw_one_workflow() {
    let eagle_eye = layover_core::diagram::mermaid::route_map_for(
        &karls_factory(),
        &Live::default(),
        &Scope::Pipeline("eagle-eye".into()),
    );

    assert!(
        eagle_eye.contains("a_azurix -- spawn --> a_eagle"),
        "{eagle_eye}"
    );
    assert!(!eagle_eye.contains("a_bob"), "{eagle_eye}");
    assert!(!eagle_eye.contains("p_devforge"), "{eagle_eye}");
}
