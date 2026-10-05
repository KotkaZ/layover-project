//! Picking up work an earlier chain set down, when a pipeline that resumes layovers fires.

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::handover::{Handover, Resumption};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_store::Journal;

/// Opens a fresh chain for each layover that has come due.
///
/// # Why a new itinerary rather than reviving the old one
///
/// The chain that booked the layover is over. Its Hops are spent, its Fuel is spent, and reviving
/// it would mean a follow-up costing the budget of the work it follows up — so the second comment
/// on a pull request would be cheaper than the first and the tenth would be free or refused,
/// depending on which rail ran out. A layover is new work about an old subject, and it is priced
/// that way.
///
/// What carries over is context, not budget: the resumed run is told which chain set this down,
/// what it was waiting for and when, the message that woke the run that set it down, and what that
/// run reported — each cut to size.
pub(super) fn resume_due(
    config: &Config,
    journal: &Journal,
    name: &PipelineName,
    now: Timestamp,
    announce: &dyn Fn(String),
) -> Result<usize, String> {
    let pipeline = config
        .pipelines
        .get(name)
        .ok_or_else(|| format!("`{name}` is not declared"))?;

    let due = journal.due(now).map_err(|error| error.to_string())?;
    // Everything still kept, up to a moment past now: a report filed an instant ago must be found.
    let everything = layover_core::cost::Window::AllTime.resolve(
        &Timestamp::now()
            .checked_add(jiff::SignedDuration::from_mins(1))
            .unwrap_or(now)
            .to_zoned(jiff::tz::TimeZone::UTC),
    );
    let mut resumed = 0;

    for layover in due {
        let resumption = Resumption {
            booked_by: layover.booked_by.clone(),
            waiting_for: layover.waiting_for.clone(),
            booked_at: layover.booked_at,
        };

        // What the run that set this down concluded. Looked up now rather than stored at booking,
        // because a run usually reports after it books.
        let reported = layover
            .run
            .as_ref()
            .and_then(|run| journal.report_for(&everything, run.as_str()).ok().flatten())
            .map(|report| {
                format!("{}\n\n{}", report.headline.trim(), report.body.trim())
                    .trim()
                    .to_owned()
            });

        let body = Handover::resumed(resumption, layover.handover.flights.clone())
            .with_progress(reported.into_iter().collect())
            .brief();

        // The layover names the agent to come back to; the pipeline only says that this factory
        // collects them. Sending to the pipeline's entry agent instead would hand a follow-up to
        // whatever happens to be first in the route map.
        let flight = Flight::new(
            ItineraryId::generate(),
            Origin::Resumed(layover.id.clone()),
            layover.agent.clone(),
            body,
            config.defaults.max_hops,
        );

        // The booking chain's values, within what this pipeline declares. The operator's choices
        // about the work were made when it was triggered, and a follow-up composed from the
        // resuming pipeline's defaults would quietly undo them. The declared set stays this
        // pipeline's, because that is what validation checked the reachable prompts against.
        let flags = pipeline.flags_carrying(&layover.flags).to_map();

        // The resumed chain belongs to this pipeline, but may use only the routes the chain that
        // set the work down could also use. Otherwise any chain could reach another workflow's
        // agents by booking a layover and waiting for this pipeline to collect it. A layover booked
        // before its scope was recorded has nothing to narrow by, and resumes as it always did.
        let within = layover
            .scope
            .as_ref()
            .map(|booked| {
                layover_core::scope::ChainScope::resuming(Some(name.clone()), booked).within
            })
            .unwrap_or_default();

        if let Err(error) =
            journal.queue(Queued::new(flight, Some(name.clone()), flags).narrowed_by(within))
        {
            announce(format!("could not resume {}: {error}", layover.id));
            continue;
        }

        // Marked resumed only after the work is queued. The other order loses the layover if the
        // queue write fails — work somebody is owed, gone with nothing to show it existed.
        let _ = journal.amend(&layover.id, layover_core::layover::Layover::resumed);
        resumed += 1;

        announce(format!(
            "resumed {} for `{}`: {}",
            layover.id, layover.agent, layover.waiting_for
        ));
    }

    Ok(resumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tower::fixture::{factory_config, temp};
    use crate::tower::trigger;
    use layover_core::agent::AgentName;

    #[test]
    fn a_due_layover_is_resumed_as_a_new_chain_with_a_fresh_budget() {
        // The chain that booked it is over: its Hops and Fuel are spent. Reviving it would make
        // the second follow-up cheaper than the first and the tenth refused.
        let root = temp("resume");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
trigger = { every = "20m" }
resumes = true
"#,
        );

        let booked_by = ItineraryId::generate();
        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                booked_by.clone(),
                "the review to land",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now.checked_sub(jiff::SignedDuration::from_hours(2))
                    .expect("in range"),
                now.checked_sub(jiff::SignedDuration::from_hours(1))
                    .expect("in range"),
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1, "the due layover became work");
        assert_ne!(
            pending[0].flight.itinerary, booked_by,
            "a resumed layover opens a new chain"
        );
        assert_eq!(pending[0].flight.hops_remaining, 4);
        assert_eq!(pending[0].pipeline, Some(PipelineName::new("follow_up")));
        assert!(
            matches!(pending[0].flight.from, Origin::Resumed(_)),
            "the resumed run is told it is picking up work set down earlier: {:?}",
            pending[0].flight.from
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_run_is_told_what_it_is_coming_back_for() {
        // Without this it is a fresh run with no idea which work item it is following up, which
        // is the failure booking a layover exists to avoid.
        let root = temp("resume-body");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "comments on pull request 41",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let body = journal.pending().expect("readable")[0].flight.body.clone();
        assert!(body.contains("comments on pull request 41"), "{body}");
        assert!(body.contains("set down"), "{body}");
        assert!(
            !body.contains("anywhere from nowhere to almost finished"),
            "a resumed layover left nothing half-done: {body}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_layover_carries_the_booking_chains_flags() {
        // The operator switched `deep` on when the work was triggered. The follow-up is the same
        // work, so it must not quietly revert to the resuming pipeline's default.
        let root = temp("resume-flags");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true

[pipelines.follow_up.flags]
deep = { default = false }
draft = { default = true }
"#,
        );

        let now = Timestamp::now();
        journal
            .book(
                layover_core::layover::Layover::book(
                    AgentName::new("worker"),
                    ItineraryId::generate(),
                    "comments on pull request 41",
                    layover_core::handover::Handover::dispatch(Vec::new()),
                    now,
                    now,
                )
                .with_flags(std::collections::BTreeMap::from([(
                    "deep".to_owned(),
                    true,
                )])),
            )
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let pending = journal.pending().expect("readable");
        assert_eq!(
            pending[0].flags.get("deep"),
            Some(&true),
            "the chain's choice"
        );
        assert_eq!(
            pending[0].flags.get("draft"),
            Some(&true),
            "a flag the booking chain never had takes the resuming pipeline's default"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_run_is_told_what_the_booking_run_was_asked_and_what_it_reported() {
        // `follower.md` promises the resumed agent learns which work item this is and what the
        // earlier chain concluded. It used to be handed only the line it was waiting for.
        let root = temp("resume-context");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        let booking_run = layover_core::RunId::generate();
        let chain = ItineraryId::generate();
        let woke = Flight::new(
            chain.clone(),
            Origin::Agent(AgentName::new("developer")),
            AgentName::new("worker"),
            "Publish work item 4821: the retry policy fix.",
            3,
        );

        journal
            .book(
                layover_core::layover::Layover::book(
                    AgentName::new("worker"),
                    chain.clone(),
                    "comments on pull request 41",
                    layover_core::handover::Handover::dispatch(vec![woke]),
                    now,
                    now,
                )
                .booked_in(booking_run.clone()),
            )
            .expect("books");
        journal
            .file(&layover_core::report::Report::new(
                booking_run,
                AgentName::new("worker"),
                chain,
                "Opened draft pull request 41",
                "Branch fix/retry-4821; tests green.",
                now.checked_sub(jiff::SignedDuration::from_mins(1))
                    .expect("in range"),
            ))
            .expect("files the report");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let body = journal.pending().expect("readable")[0].flight.body.clone();
        assert!(body.contains("work item 4821"), "what woke it: {body}");
        assert!(
            body.contains("Opened draft pull request 41"),
            "its report: {body}"
        );
        assert!(
            body.contains("fix/retry-4821"),
            "the report's detail: {body}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_chain_is_held_to_what_the_booking_chain_could_reach() {
        // Any agent may book a layover and a resuming pipeline collects them all. Without the
        // narrowing, work an Eagle Eye chain set down would come back with the follow-up
        // pipeline's routes, including every DevForge edge.
        let root = temp("resume-narrowed");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        let book = |scope: Option<layover_core::scope::ChainScope>| {
            let layover = layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "later",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
            );
            let layover = match scope {
                Some(scope) => layover.booked_within(scope),
                None => layover,
            };
            journal.book(layover).expect("books");
        };
        book(Some(layover_core::scope::ChainScope::of(Some(
            PipelineName::new("eagle-eye"),
        ))));
        book(None);

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 2);
        assert!(
            pending
                .iter()
                .all(|queued| { queued.pipeline == Some(PipelineName::new("follow_up")) })
        );
        assert!(
            pending.iter().any(|queued| queued.within
                == std::collections::BTreeSet::from([Some(PipelineName::new("eagle-eye"))])),
            "{pending:?}"
        );
        assert!(
            pending.iter().any(|queued| queued.within.is_empty()),
            "a layover booked before scopes were recorded resumes as it always did"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_layover_that_is_not_due_yet_is_left_alone() {
        let root = temp("resume-early");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "later",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now.checked_add(jiff::SignedDuration::from_hours(2))
                    .expect("in range"),
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        assert!(journal.pending().expect("readable").is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_layover_is_not_resumed_twice() {
        let root = temp("resume-once");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "once",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");
        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        assert_eq!(
            journal.pending().expect("readable").len(),
            1,
            "a layover picked up twice is one follow-up done twice"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_ordinary_schedule_never_collects_booked_work() {
        // Only a pipeline that declares `resumes` goes looking. Otherwise a factory's hourly
        // sweep would quietly start following up other people's work.
        let root = temp("resume-none");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.build]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "nobody collects this",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
            ))
            .expect("books");

        // What the loop does for a pipeline that does not resume.
        trigger(&config, &journal, &PipelineName::new("build")).expect("queues fresh work");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1);
        assert!(
            !pending[0].flight.body.contains("set down"),
            "an ordinary tick opens fresh work, not a follow-up: {}",
            pending[0].flight.body
        );
        assert_eq!(
            journal.due(now).expect("readable").len(),
            1,
            "the layover is still owed, and still visible"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
