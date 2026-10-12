//! A run deletes the learnings that lapsed more than `[learnings] keep_lapsed_days` ago — its own
//! agent's and every other's — and keeps the rest.

use jiff::{SignedDuration, Timestamp};
use layover_core::learning::{Impact, Learnings, Proposal, State};
use layover_store::Journal;

mod runs;
use runs::{Temp, factory, human, sleeps};

fn factory_text(learnings: &str) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
timeout_sec = 60

[runners.quick]
command = {}

[agents.worker]
prompt = "work"
entry = true

{learnings}
"#,
        sleeps(0)
    )
}

fn journal(temp: &Temp) -> Journal {
    Journal::open(temp.0.join(".layover").join("journal")).expect("opens")
}

fn days_ago(days: i64) -> Timestamp {
    Timestamp::now() - SignedDuration::from_hours(days * 24)
}

/// One learning per `(agent, text, lapsed days ago)`; `None` leaves it provisional with one run
/// left, so the next run of its agent lapses it.
fn seed(temp: &Temp, learnings: &[(&str, &str, Option<i64>)]) {
    let mut seeded = Learnings::new();
    for (agent, text, _) in learnings {
        seeded.propose(&Proposal::new(
            (*agent).into(),
            *text,
            Impact::Medium,
            days_ago(30),
        ));
    }
    let mut entries: Vec<_> = seeded.all().cloned().collect();
    for (learning, (_, _, lapsed)) in entries.iter_mut().zip(learnings) {
        match lapsed {
            Some(days) => {
                learning.state = State::Lapsed;
                learning.runs_left = 0;
                learning.lapsed_at = Some(days_ago(*days));
            }
            None => learning.runs_left = 1,
        }
    }
    journal(temp)
        .save_learnings(&Learnings::from_entries(entries))
        .expect("saves");
}

fn texts(temp: &Temp) -> Vec<String> {
    journal(temp)
        .learnings()
        .expect("reads")
        .all()
        .map(|learning| learning.text.clone())
        .collect()
}

#[test]
fn a_run_deletes_what_lapsed_longer_ago_than_the_factory_keeps_it() {
    let temp = Temp::new("forget-default");
    let factory = factory(&temp, &factory_text(""));
    seed(
        &temp,
        &[
            ("reviewer", "lapsed three weeks ago", Some(21)),
            ("reviewer", "lapsed two days ago", Some(2)),
            ("worker", "still in use", None),
        ],
    );

    factory.drain(vec![human("worker")], |_| {}, |_, _| {});

    assert_eq!(
        texts(&temp),
        ["lapsed two days ago", "still in use"],
        "fourteen days by default; another agent's learnings go too"
    );
}

#[test]
fn with_no_days_kept_a_learning_is_deleted_by_the_run_that_lapses_it() {
    let temp = Temp::new("forget-now");
    let factory = factory(&temp, &factory_text("[learnings]\nkeep_lapsed_days = 0\n"));
    seed(
        &temp,
        &[
            ("worker", "runs out on this run", None),
            ("reviewer", "lapsed a minute ago", Some(0)),
        ],
    );

    factory.drain(vec![human("worker")], |_| {}, |_, _| {});

    assert!(texts(&temp).is_empty(), "{:?}", texts(&temp));
}
