//! Return paths, and the points where edges meet their nodes.

use super::*;

#[test]
fn an_edge_pointing_back_towards_the_entry_is_marked_as_a_return_path() {
    let layout = Layout::build(&factory(), &Live::default());

    let back = layout
        .edges
        .iter()
        .find(|edge| edge.from == "a_tester" && edge.to == "a_developer")
        .expect("the review loop exists");

    assert!(back.back, "tester sits right of developer, so this returns");
    assert_eq!(back.style, EdgeStyle::Joined);
    assert_eq!(back.label.as_deref(), Some("all"));
}

#[test]
fn return_paths_sit_inside_the_reported_extent_too() {
    // The first version of this reserved height for nodes only, and every review loop in the
    // reference factory was drawn below the viewBox and clipped away. Found by rendering it
    // and looking, which is the only way that class of bug ever shows up.
    let layout = Layout::build(&factory(), &Live::default());

    let returns: Vec<&Edge> = layout.edges.iter().filter(|edge| edge.back).collect();
    assert!(!returns.is_empty(), "the factory has a review loop to test");

    for edge in returns {
        let floor = edge.floor.expect("a return path is given a lane");
        assert!(
            floor <= layout.height,
            "{} -> {} dips to {floor} but the drawing is only {} tall",
            edge.from,
            edge.to,
            layout.height
        );
    }
}

#[test]
fn several_returns_to_one_agent_climb_at_different_points() {
    // The normal shape of a review loop: a tester and a reviewer both report back to the
    // developer. Sharing one gutter put both curves on the same vertical line and stacked
    // their labels on top of each other, which is what the whole diagram looked like.
    let layout = Layout::build(&review_loop(), &Live::default());
    let returns: Vec<f64> = layout
        .edges
        .iter()
        .filter(|edge| edge.back && edge.to == "a_dev")
        .filter_map(|edge| edge.gutter)
        .collect();

    assert!(returns.len() >= 2, "the factory has a review loop");
    for pair in returns.windows(2) {
        assert!(
            (pair[0] - pair[1]).abs() > 1.0,
            "two returns share a gutter: {returns:?}"
        );
    }

    let hooks: Vec<f64> = layout
        .edges
        .iter()
        .filter(|edge| edge.back && edge.to == "a_dev")
        .filter_map(|edge| edge.hook_x)
        .collect();
    for pair in hooks.windows(2) {
        assert!(
            (pair[0] - pair[1]).abs() > 1.0,
            "two returns hook in at the same point: {hooks:?}"
        );
    }
}

#[test]
fn a_return_path_hooks_inside_the_agent_it_returns_to() {
    let layout = Layout::build(&review_loop(), &Live::default());
    let developer = layout.node("a_dev").expect("dev");

    for edge in layout.edges.iter().filter(|edge| edge.back) {
        let hook = edge.hook_x.expect("a return path is given a hook");
        assert!(
            hook > developer.x && hook < developer.x + developer.w,
            "the hook must land on the node, not beside it: {hook}"
        );
    }
}

#[test]
fn two_return_paths_are_given_lanes_of_their_own() {
    // Drawn at the same depth they would overlap, and a review loop is the structure on a
    // route map most worth being able to follow with a finger.
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.dev]
        prompt = "develop"

        [agents.tester]
        prompt = "test"

        [agents.reviewer]
        prompt = "review"

        [pipelines.go]
        entry = "dev"

        [[routes]]
        from = "dev"
        to = ["tester", "reviewer"]

        [[routes]]
        from = ["tester", "reviewer"]
        to = "dev"
        join = "all"
        "#,
    );

    let layout = Layout::build(&config, &Live::default());
    let mut floors: Vec<f64> = layout
        .edges
        .iter()
        .filter(|edge| edge.back)
        .filter_map(|edge| edge.floor)
        .collect();
    floors.sort_by(f64::total_cmp);

    assert_eq!(floors.len(), 2);
    assert!(
        (floors[1] - floors[0]).abs() > 1.0,
        "the two loops share a lane: {floors:?}"
    );
}

#[test]
fn edges_leaving_one_node_meet_it_at_different_points() {
    // They all left from the centre, so several going to different places overlapped for
    // their first stretch and only separated after they had already crossed.
    let layout = Layout::build(&factory(), &Live::default());
    let leaving: Vec<f64> = layout
        .edges
        .iter()
        .filter(|edge| edge.from == "a_developer" && !edge.back)
        .map(|edge| edge.from_y)
        .collect();

    assert!(leaving.len() >= 2, "the developer fans out");
    for pair in leaving.windows(2) {
        assert!(
            (pair[0] - pair[1]).abs() > 1.0,
            "two edges share an exit point: {leaving:?}"
        );
    }
}

#[test]
fn a_node_with_one_edge_still_meets_it_in_the_middle() {
    let layout = Layout::build(&factory(), &Live::default());
    let analyst = layout.node("a_analyst").expect("analyst");
    let only = layout
        .edges
        .iter()
        .find(|edge| edge.from == "a_analyst" && !edge.back)
        .expect("analyst sends somewhere");

    assert!((only.from_y - analyst.centre().1).abs() < 0.001);
}
