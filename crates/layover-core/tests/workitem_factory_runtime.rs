//! How the reference factory is *triggered* and *bounded*.
//!
//! Covers the parts of `examples/workitem-factory/` that decide what a run actually receives —
//! pipelines, schedules, flags and composed prompts — and the Hops and run-cap arithmetic that
//! its README documents. The mesh shape itself is in `workitem_factory.rs`.

mod common;

use std::collections::BTreeMap;

use common::{REVIEW_CYCLES, factory, prompts};
use layover_core::prompt::resolve;
use layover_core::{Itinerary, ItineraryId, PipelineName, Schedule, Trigger};

// ── Pipelines ─────────────────────────────────────────────────────────────────────────────

#[test]
fn feature_work_is_manual_and_review_work_is_scheduled() {
    // A clock should not decide that a feature ought to be built; nobody should have to remember
    // to review their own pull requests.
    let config = factory();

    assert_eq!(
        config.pipelines[&PipelineName::from("development")].trigger,
        Trigger::Manual
    );

    let review = &config.pipelines[&PipelineName::from("review-bot")];
    assert_eq!(
        review.trigger,
        Trigger::Scheduled(Schedule::Every(std::time::Duration::from_secs(3_600))),
        "the review bot runs hourly"
    );
    assert_eq!(review.entry.as_str(), "pr_scanner");
}

#[test]
fn both_pipelines_are_entry_points_and_nothing_else_is() {
    let config = factory();
    let mut entries: Vec<String> = config.entry_agents().map(ToString::to_string).collect();
    entries.sort();

    assert_eq!(entries, ["analyst", "pr_scanner"]);
}

#[test]
fn the_hourly_schedule_is_slower_than_a_run_may_take() {
    // Runs are reentrant: a schedule that outruns its own work piles up concurrent copies rather
    // than queueing. This is the invariant behind the validation warning.
    let config = factory();
    let interval = config.pipelines[&PipelineName::from("review-bot")]
        .trigger
        .schedule()
        .and_then(Schedule::interval)
        .expect("the review bot has a fixed interval");

    assert!(
        interval.as_secs() >= config.defaults.timeout_sec,
        "an hourly schedule must outlast the {}s run timeout",
        config.defaults.timeout_sec
    );
}

#[test]
fn every_pipeline_declares_the_flags_its_prompts_test() {
    let config = factory();
    let mut declared: Vec<&str> = config.declared_flags().collect();
    declared.sort_unstable();
    declared.dedup();

    assert_eq!(declared, ["deep_analysis", "draft_pr", "run_e2e"]);
}

#[test]
fn a_typo_at_trigger_time_is_refused_rather_than_ignored() {
    let config = factory();
    let pipeline = &config.pipelines[&PipelineName::from("development")];

    let overrides = BTreeMap::from([("run_e2ee".to_owned(), true)]);
    assert!(
        pipeline.flags_for_run(&overrides).is_err(),
        "an undeclared flag must not silently do nothing"
    );
}

// ── Composed prompts ──────────────────────────────────────────────────────────────────────

#[test]
fn the_tester_prompt_swaps_branches_on_the_e2e_flag() {
    let config = factory();
    let pipeline = &config.pipelines[&PipelineName::from("development")];
    let source = prompts();

    let off = pipeline
        .flags_for_run(&BTreeMap::new())
        .expect("defaults are always valid");
    let on = pipeline
        .flags_for_run(&BTreeMap::from([("run_e2e".to_owned(), true)]))
        .expect("declared flag");

    let without = resolve(&source, "tester.md", &off).expect("resolves with the flag off");
    let with = resolve(&source, "tester.md", &on).expect("resolves with the flag on");

    assert!(without.contains("Local suite only"));
    assert!(!without.contains("End-to-end suite"));

    assert!(with.contains("End-to-end suite"));
    assert!(!with.contains("Local suite only"));

    // Both branches keep the shared instructions.
    for text in [&without, &with] {
        assert!(text.contains("You are the tester."));
        assert!(text.contains("explicit verdict"));
        assert!(
            !text.contains("@include"),
            "an agent must never see Layover's own directive syntax"
        );
    }
}

#[test]
fn the_publisher_opens_a_draft_by_default() {
    // Publishing without a human ever looking is the one irreversible step, so the default is the
    // cautious one and turning it off is a deliberate act.
    let config = factory();
    let pipeline = &config.pipelines[&PipelineName::from("development")];
    let flags = pipeline
        .flags_for_run(&BTreeMap::new())
        .expect("defaults are always valid");

    assert_eq!(flags.get("draft_pr"), Some(true));

    let text = resolve(&prompts(), "publisher.md", &flags).expect("resolves");
    assert!(text.contains("Draft pull request"));
}

#[test]
fn an_agent_with_no_conditional_sections_resolves_to_its_file() {
    let config = factory();
    let flags = config.pipelines[&PipelineName::from("development")]
        .flags_for_run(&BTreeMap::new())
        .expect("defaults are always valid");

    let text = resolve(&prompts(), "reviewer.md", &flags).expect("resolves");

    assert!(text.contains("You are the reviewer."));
    assert!(!text.contains("@include"));
}

// ── Bounds: the arithmetic the example's README documents ─────────────────────────────────

/// Spends one hop, reporting whether the chain survived.
fn fly(itinerary: &Itinerary, hops: &mut u32) -> bool {
    match itinerary.authorize_send(*hops) {
        Ok(next) => {
            *hops = next;
            true
        }
        Err(_) => false,
    }
}

/// Walks the factory's longest path and counts test/review cycles that can still be published.
///
/// `lead_in` is the number of flights spent before the developer's first run: four for the manual
/// pipeline, five for the scheduled one, which spends an extra flight getting from `pr_scanner`
/// to `analyst`.
///
/// A cycle the developer cannot follow with a flight to the publisher does not count: the work is
/// finished and stranded, which is the failure the hop budget exists to prevent.
fn publishable_review_cycles(max_hops: u32, lead_in: u32) -> u32 {
    let itinerary = Itinerary::new(ItineraryId::generate(), max_hops, 1_000.0, 10_000);
    let mut hops = itinerary.initial_hops();

    // The trigger is flight 1; the rest of the lead-in is spent here.
    for _ in 0..lead_in - 1 {
        if !fly(&itinerary, &mut hops) {
            return 0;
        }
    }

    let mut publishable = 0;
    for cycle in 1..=100 {
        // developer → tester and reviewer, then their verdicts → developer.
        if !fly(&itinerary, &mut hops) || !fly(&itinerary, &mut hops) {
            return publishable;
        }

        // developer → publisher, if there is anything left to fly it.
        if itinerary.authorize_send(hops).is_err() {
            return publishable;
        }
        publishable = cycle;
    }

    publishable
}

/// Flights spent before the developer's first run, per entry path.
const MANUAL_LEAD_IN: u32 = 4;
const SCHEDULED_LEAD_IN: u32 = 5;

#[test]
fn the_configured_hop_budget_allows_eight_cycles_from_either_entry() {
    let config = factory();

    assert_eq!(config.defaults.max_hops, 22);
    assert_eq!(
        publishable_review_cycles(config.defaults.max_hops, MANUAL_LEAD_IN),
        REVIEW_CYCLES,
        "manual path: 2N + 5 flights"
    );
    assert_eq!(
        publishable_review_cycles(config.defaults.max_hops, SCHEDULED_LEAD_IN),
        REVIEW_CYCLES,
        "scheduled path: 2N + 6 flights, which is what 22 is sized for"
    );
}

#[test]
fn the_default_hop_budget_leaves_no_room_to_fix_anything() {
    // The reason this example overrides `max_hops` at all. At the default the developer gets one
    // pass through the tester and the reviewer, and the first rejection ends the itinerary with
    // half-repaired work in the shared workspace.
    assert_eq!(
        publishable_review_cycles(8, MANUAL_LEAD_IN),
        1,
        "the default budget permits the happy path and nothing else"
    );
    assert_eq!(
        publishable_review_cycles(8, SCHEDULED_LEAD_IN),
        1,
        "and the scheduled path is one flight worse off still"
    );
}

#[test]
fn the_hop_budget_is_the_documented_function_of_the_cycle_count() {
    for cycles in 1..=REVIEW_CYCLES {
        for (lead_in, fixed) in [(MANUAL_LEAD_IN, 5), (SCHEDULED_LEAD_IN, 6)] {
            let minimum = 2 * cycles + fixed;

            assert_eq!(
                publishable_review_cycles(minimum, lead_in),
                cycles,
                "{minimum} hops should buy {cycles} cycles on the lead-in-{lead_in} path"
            );
            assert_eq!(
                publishable_review_cycles(minimum - 1, lead_in),
                cycles - 1,
                "one hop short must cost exactly one cycle"
            );
        }
    }
}

#[test]
fn the_run_cap_covers_the_whole_loop_without_relying_on_cost_reporting() {
    let config = factory();
    let mut itinerary = Itinerary::new(
        ItineraryId::generate(),
        config.defaults.max_hops,
        config.defaults.fuel_usd,
        config.defaults.max_runs,
    );

    // 3N + 7 at N = 8: the scanner once, the analyst twice, both helpers, the developer nine
    // times, the tester and the reviewer eight times each, and the publisher once.
    let expected_runs = 3 * REVIEW_CYCLES + 7;
    assert_eq!(expected_runs, 31);

    for run in 0..expected_runs {
        itinerary
            .record_run_started()
            .unwrap_or_else(|error| panic!("run {run} must be permitted: {error}"));
        // No runner in this factory is assumed to report cost.
        itinerary.note_unreported_cost();
    }

    assert!(
        itinerary.has_cost_reporting_gap(),
        "the Tower must be able to surface silent metering"
    );
    assert!(
        itinerary.runs_remaining() > 0,
        "the cap must leave headroom rather than land exactly on the expected count"
    );
}
