//! Columns follow reading order: a depth-first walk from the way in, in declaration order.
//!
//! The fixture is the production factory these were found on, routes as it declares them: one
//! workflow whose analyst may send straight to agents several phases on, a sweep that spawns a
//! reviewer per pull request, and a monitor that shares only the messenger.

use super::*;

/// The production factory's routing, agents reduced to what the layout reads.
pub(super) fn production() -> Config {
    production_with(
        "[\"investigator\", \"telemetry\", \"reviewer\", \"developer\", \"ado-operator\"]",
    )
}

/// The same factory, with the analyst's routes listed in another order.
pub(super) fn production_with(analyst_sends_to: &str) -> Config {
    config(&format!(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.analyst]
        prompt = "analyse"
        [agents.investigator]
        prompt = "investigate"
        [agents.telemetry]
        prompt = "measure"
        [agents.reviewer]
        prompt = "review"
        [agents.developer]
        prompt = "develop"
        [agents.tester]
        prompt = "test"
        [agents.ado-operator]
        prompt = "operate"
        [agents.messenger]
        prompt = "tell"
        [agents.platform-monitor]
        prompt = "monitor"

        [pipelines.devforge]
        entry = "analyst"
        [pipelines.eagle-eye]
        entry = "ado-operator"
        [pipelines.chromium-webrtc-monitor]
        entry = "platform-monitor"

        [[routes]]
        from = "analyst"
        to = {analyst_sends_to}
        pipelines = "devforge"
        [[routes]]
        from = ["investigator", "telemetry"]
        to = "analyst"
        pipelines = "devforge"
        [[routes]]
        from = "reviewer"
        to = ["analyst", "developer"]
        pipelines = "devforge"
        [[routes]]
        from = "developer"
        to = ["analyst", "tester", "ado-operator"]
        pipelines = "devforge"
        [[routes]]
        from = "tester"
        to = ["developer", "reviewer", "ado-operator"]
        pipelines = "devforge"
        [[routes]]
        from = "ado-operator"
        to = ["developer", "messenger", "analyst"]
        pipelines = "devforge"

        [[routes]]
        from = "ado-operator"
        to = "reviewer"
        mode = "spawn"
        pipelines = "eagle-eye"
        [[routes]]
        from = "reviewer"
        to = ["ado-operator", "investigator", "telemetry", "tester"]
        pipelines = "eagle-eye"
        [[routes]]
        from = ["investigator", "telemetry", "tester"]
        to = "reviewer"
        pipelines = "eagle-eye"

        [[routes]]
        from = "platform-monitor"
        to = "messenger"
        pipelines = "chromium-webrtc-monitor"
        "#
    ))
}

/// Each column's agents, by name, left to right, without the column of ways in.
pub(super) fn columns(layout: &Layout) -> Vec<Vec<String>> {
    let mut by_layer: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for node in layout
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Agent)
    {
        by_layer
            .entry(node.layer)
            .or_default()
            .push(node.label.clone());
    }
    by_layer
        .into_values()
        .map(|mut names| {
            names.sort();
            names
        })
        .collect()
}

fn names(columns: &[&[&str]]) -> Vec<Vec<String>> {
    columns
        .iter()
        .map(|column| column.iter().map(|name| (*name).to_owned()).collect())
        .collect()
}

pub(super) fn devforge(config: &Config) -> Layout {
    Layout::scoped(
        config,
        &Live::default(),
        &Scope::Pipeline("devforge".into()),
    )
}

#[test]
fn a_workflow_with_shortcuts_reads_left_to_right_along_its_main_path() {
    // The analyst may send straight to the reviewer, the developer and the operator, for rare
    // messages. Breadth-first columns put all three one hop in, in one column with the analyst's
    // two consults, and the phase order the work follows vanished from the map.
    let layout = devforge(&production());
    let drawn = layout
        .edges
        .iter()
        .filter(|edge| edge.style != EdgeStyle::Entry)
        .map(|edge| usize::from(edge.both) + 1)
        .sum::<usize>();

    assert_eq!(
        drawn, 18,
        "the fixture is the 18 routes the factory declares"
    );
    assert_eq!(
        columns(&layout),
        names(&[
            &["analyst"],
            &["investigator", "reviewer", "telemetry"],
            &["developer"],
            &["tester"],
            &["ado-operator"],
            &["messenger"],
        ])
    );
}

#[test]
fn the_order_routes_are_declared_in_shapes_the_columns() {
    // Declaration order is part of the picture, deliberately: it is what lets a factory's map follow
    // its main path. The same routes, the analyst's listed operator first, read in another order.
    let layout = devforge(&production_with(
        "[\"ado-operator\", \"investigator\", \"telemetry\", \"reviewer\", \"developer\"]",
    ));

    assert_eq!(
        columns(&layout),
        names(&[
            &["analyst"],
            &["ado-operator", "investigator", "telemetry"],
            &["developer", "messenger"],
            &["tester"],
            &["reviewer"],
        ])
    );
}

#[test]
fn a_route_closing_a_loop_is_drawn_back_and_its_partner_merged_with_it() {
    let layout = devforge(&production());
    let edge = |from: &str, to: &str| {
        layout
            .edges
            .iter()
            .find(|edge| {
                (edge.from == agent_id(&from.into()) && edge.to == agent_id(&to.into()))
                    || (edge.both
                        && edge.from == agent_id(&to.into())
                        && edge.to == agent_id(&from.into()))
            })
            .unwrap_or_else(|| panic!("{from} -> {to} is drawn"))
    };

    // Set aside as back edges, then merged with the forward route they answer.
    for (from, to) in [
        ("investigator", "analyst"),
        ("reviewer", "analyst"),
        ("tester", "developer"),
        ("ado-operator", "developer"),
        ("ado-operator", "analyst"),
    ] {
        let drawn = edge(from, to);
        assert!(drawn.both && !drawn.back, "{from} -> {to}: {drawn:?}");
    }
    // The one that answers nothing stays a route back.
    assert!(edge("tester", "reviewer").back);
}

#[test]
fn a_sweep_that_spawns_its_reviewer_reads_spawner_reviewer_then_consults() {
    // Breadth-first distance stops at a spawn, so the reviewer's consults were unreachable and
    // landed in the first column beside the sweep's own entry.
    let layout = Layout::scoped(
        &production(),
        &Live::default(),
        &Scope::Pipeline("eagle-eye".into()),
    );

    assert_eq!(
        columns(&layout),
        names(&[
            &["ado-operator"],
            &["reviewer"],
            &["investigator", "telemetry", "tester"],
        ])
    );
}

#[test]
fn an_agent_two_workflows_reach_at_different_depths_takes_the_deeper_column() {
    // On the whole-factory map every way in seeds the walk, by pipeline name. The fixer is a way
    // in of its own, but the scout's workflow reaches it a column later, and it is drawn there.
    let config = config(
        r#"
        [layover]
        work_dir = "work"
        [defaults]
        runner = "claude"
        [runners.claude]
        command = ["claude", "-p"]
        [agents.scout]
        prompt = "scout"
        [agents.fixer]
        prompt = "fix"
        [agents.shipper]
        prompt = "ship"
        [pipelines.a-scouting]
        entry = "scout"
        [pipelines.b-fixing]
        entry = "fixer"
        [[routes]]
        from = "scout"
        to = "fixer"
        [[routes]]
        from = "fixer"
        to = "shipper"
        "#,
    );
    let layout = Layout::build(&config, &Live::default());

    assert_eq!(
        columns(&layout),
        names(&[&["scout"], &["fixer"], &["shipper"]])
    );
}

#[test]
fn an_edge_that_skips_columns_crosses_each_one_clear_of_its_boxes() {
    let layout = devforge(&production());
    let operator = layout
        .edges
        .iter()
        .find(|edge| {
            edge.from == "a_analyst" && edge.to == "a_ado-operator"
                || edge.from == "a_ado-operator" && edge.to == "a_analyst"
        })
        .expect("analyst and operator are joined");

    // Columns 2, 3 and 4 lie between, each crossed from its left edge to its right.
    assert_eq!(operator.via.len(), 6, "{:?}", operator.via);
    for pair in operator.via.chunks(2) {
        let ((enter, y), (leave, same)) = (pair[0], pair[1]);
        assert!((y - same).abs() < 0.001 && leave > enter);
        for node in &layout.nodes {
            let beside = node.x < leave && node.x + node.w > enter;
            assert!(
                !beside || y < node.y || y > node.y + node.h,
                "the crossing at {y} runs through {}",
                node.id
            );
        }
    }
}

#[test]
fn hop_depth_is_said_in_the_tooltip_now_that_columns_do_not() {
    // The developer is four columns along but one hop in: the analyst may send to it directly.
    // The tooltip carries the hop check's own arithmetic, so the column cannot be misread for it.
    let layout = devforge(&production());
    let developer = layout.node("a_developer").expect("developer");

    assert_eq!(developer.layer, 3);
    let tooltip = developer.tooltip.as_deref().unwrap_or_default();
    assert!(
        tooltip.contains("1 hop from devforge's entry: woken by flight 2 at the earliest"),
        "{tooltip}"
    );
    let analyst = layout.node("a_analyst").expect("analyst");
    assert!(
        analyst
            .tooltip
            .as_deref()
            .unwrap_or_default()
            .contains("devforge's entry: woken by its first flight")
    );

    let sweep = Layout::scoped(
        &production(),
        &Live::default(),
        &Scope::Pipeline("eagle-eye".into()),
    );
    let said = |id: &str| {
        sweep
            .node(id)
            .and_then(|node| node.tooltip.clone())
            .unwrap_or_default()
    };
    assert!(said("a_reviewer").contains("starts a chain of its own: it is spawned"));
    assert!(
        said("a_investigator").contains("1 hop into a chain spawned at reviewer"),
        "{}",
        said("a_investigator")
    );
}

#[test]
fn a_return_path_runs_under_the_columns_it_spans_not_the_whole_map() {
    // Every return used to drop below the whole drawing, so a loop between two neighbouring
    // columns at the far right was the largest shape on the map. It now runs just below the
    // columns it spans; here that is well above the bottom of the tall column before them.
    let config = config(
        r#"
        [layover]
        work_dir = "work"
        [defaults]
        runner = "claude"
        [runners.claude]
        command = ["claude", "-p"]
        [agents.lead]
        prompt = "lead"
        [agents.one]
        prompt = "one"
        [agents.two]
        prompt = "two"
        [agents.three]
        prompt = "three"
        [agents.four]
        prompt = "four"
        [agents.maker]
        prompt = "make"
        [agents.checker]
        prompt = "check"
        [pipelines.go]
        entry = "lead"
        [[routes]]
        from = "lead"
        to = ["one", "two", "three", "four"]
        [[routes]]
        from = "one"
        to = "maker"
        [[routes]]
        from = "maker"
        to = "checker"
        [[routes]]
        from = "checker"
        to = "maker"
        join = "all"
        "#,
    );
    let layout = Layout::build(&config, &Live::default());
    let back = layout
        .edges
        .iter()
        .find(|edge| edge.back)
        .expect("the checker answers the maker");
    let floor = back.floor.expect("a return path is given a lane");
    let column = layout.node("a_one").expect("one").layer;
    let tall = layout
        .nodes
        .iter()
        .filter(|node| node.layer == column)
        .map(|node| node.y + node.h)
        .fold(0.0_f64, f64::max);
    let near = ["a_maker", "a_checker"]
        .iter()
        .filter_map(|id| layout.node(id))
        .map(|node| node.y + node.h)
        .fold(0.0_f64, f64::max);

    assert!(floor > near, "below what it spans: {floor} vs {near}");
    assert!(
        floor < tall,
        "but not under the tall column it does not span: {floor} vs {tall}"
    );
}
