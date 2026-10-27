//! Dismissals: a stalled or failed chain somebody dealt with keeps its state, and says so, only
//! while it still ends on the run that was dismissed.

use super::*;

fn dismissed(record: &RunRecord, note: Option<&str>) -> Dismissal {
    Dismissal {
        itinerary: record.itinerary.clone(),
        run: record.run.clone(),
        by: "karl".to_owned(),
        at: Timestamp::now(),
        note: note.map(str::to_owned),
    }
}

#[test]
fn a_dismissed_failed_chain_stays_failed_and_says_who_dismissed_it() {
    // Somebody fixed it by hand. What happened stays on record; that it needs nobody is said
    // beside it, not by pretending it finished.
    let id = chain();
    let wolf = ended(run(&id, "wolf", Outcome::Failed, 0.30, true), 2);
    let dismissals = vec![dismissed(&wolf, Some("merged the fix by hand"))];
    let records = vec![
        ended(run(&id, "bob", Outcome::Succeeded, 0.10, true), 1),
        wolf.clone(),
    ];

    let built = itineraries(
        &records,
        Context {
            dismissals: &dismissals,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Failed);
    assert!(
        built[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .starts_with("`wolf` failed"),
        "{:?}",
        built[0].detail
    );
    let said = built[0].dismissed.as_ref().expect("dismissed");
    assert_eq!(said.run_id, wolf.run.as_str());
    assert_eq!(said.by, "karl");
    assert_eq!(said.note.as_deref(), Some("merged the fix by hand"));
}

#[test]
fn a_dismissal_of_one_failure_does_not_hide_the_next() {
    // Dismissed, then restarted, then failed again: nobody has seen the second failure.
    let id = chain();
    let first = ended(run(&id, "wolf", Outcome::Failed, 0.30, true), 1);
    let dismissals = vec![dismissed(&first, None)];
    let records = vec![
        first,
        ended(run(&id, "wolf", Outcome::Failed, 0.30, true), 9),
    ];

    let built = itineraries(
        &records,
        Context {
            dismissals: &dismissals,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Failed);
    assert_eq!(built[0].dismissed, None, "the later failure needs a person");
}

#[test]
fn a_dismissed_stalled_chain_stays_stalled() {
    let id = chain();
    let reviewer = ended(run(&id, "reviewer", Outcome::Succeeded, 0.10, true), 2);
    let stalls = vec![Stall::new(
        id.clone(),
        AgentName::new("publisher"),
        vec![AgentName::new("tester")],
        1,
        Timestamp::now(),
    )];
    let dismissals = vec![dismissed(&reviewer, None)];
    let records = vec![
        ended(run(&id, "tester", Outcome::Succeeded, 0.10, true), 1),
        reviewer,
    ];

    let built = itineraries(
        &records,
        Context {
            stalls: &stalls,
            dismissals: &dismissals,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Stalled);
    assert!(built[0].dismissed.is_some());
}

#[test]
fn a_dismissal_is_not_shown_on_a_chain_that_no_longer_needs_one() {
    // Dismissed and then continued anyway: `finished`, and the dismissal is moot. Another chain's
    // dismissal never applies.
    let id = chain();
    let wolf = run(&id, "wolf", Outcome::Failed, 0.30, true);
    let mut continuation = run(&chain(), "bob", Outcome::Succeeded, 0.10, true);
    continuation.continues = Some(id.clone());
    let elsewhere = run(&chain(), "wolf", Outcome::Failed, 0.30, true);
    let dismissals = vec![dismissed(&wolf, None)];

    let built = itineraries(
        &[wolf, continuation, elsewhere.clone()],
        Context {
            dismissals: &dismissals,
            ..Context::default()
        },
    );

    let listed = |id: &ItineraryId| {
        built
            .iter()
            .find(|chain| chain.itinerary_id == id.as_str())
            .expect("listed")
    };
    assert_eq!(listed(&id).state, ItineraryState::Finished);
    assert_eq!(listed(&id).dismissed, None);
    assert_eq!(listed(&elsewhere.itinerary).state, ItineraryState::Failed);
    assert_eq!(listed(&elsewhere.itinerary).dismissed, None);
}
