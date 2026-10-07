//! `GET /upcoming`: when each schedule next fires, which tick picks up each layover, and the ticks
//! that were skipped — and, with no Tower in the process, that the times are not known.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::flight::ItineraryId;
use layover_core::handover::Handover;
use layover_core::layover::Layover;
use layover_core::pipeline::PipelineName;
use layover_core::scope::ChainScope;
use layover_core::skip::Skip;
use layover_dashboard::{Dashboard, DashboardState, Guard, Timetable, router};
use layover_store::{History, Journal};
use serde_json::Value;
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true

[agents.follower]
prompt = "follow"

[pipelines.development]
entry = "analyst"

[pipelines.sweep]
entry = "analyst"
trigger = { every = "1h" }

[pipelines.poll]
entry = "analyst"
trigger = { every = "1m" }
overlap = "allow"

[pipelines.follow_up]
entry = "follower"
trigger = { every = "45m" }
resumes = true
"#;

/// A clock that says what a test tells it to.
#[derive(Debug, Default)]
struct Fixed {
    next: Vec<(&'static str, Timestamp)>,
    working: Vec<&'static str>,
}

impl Timetable for Fixed {
    fn next_due(&self, pipeline: &PipelineName) -> Option<Timestamp> {
        self.next
            .iter()
            .find(|(name, _)| *name == pipeline.as_str())
            .map(|(_, at)| *at)
    }

    fn working(&self) -> BTreeSet<PipelineName> {
        self.working
            .iter()
            .map(|name| PipelineName::new(*name))
            .collect()
    }
}

struct Factory {
    root: PathBuf,
    journal: Arc<Journal>,
}

impl Factory {
    fn new(label: &str, config: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-upcoming-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), config).expect("writes config");
        let journal =
            Arc::new(Journal::open(root.join(".layover").join("journal")).expect("opens"));
        Self { root, journal }
    }

    fn dashboard(&self) -> Dashboard {
        let layover = self.root.join(".layover");
        Dashboard::new(DashboardState {
            config_path: self.root.join("layover.toml"),
            history: History::open(layover.join("history")).expect("opens history"),
            journal: Arc::clone(&self.journal),
            ground_stop: layover.join("ground-stop"),
        })
    }

    fn clocked(&self, clock: Fixed) -> Dashboard {
        self.dashboard()
            .timetable("the Tower in `layover serve` (process 7)", Arc::new(clock))
    }

    fn set_down(&self, due: Timestamp, from: Option<&str>) {
        let layover = Layover::book(
            AgentName::new("follower"),
            ItineraryId::generate(),
            "comments on pull request 41",
            Handover::dispatch(Vec::new()),
            Timestamp::now(),
            due,
        );
        let layover = match from {
            Some(pipeline) => layover.booked_within(ChainScope {
                pipeline: Some(PipelineName::new(pipeline)),
                ..ChainScope::default()
            }),
            None => layover,
        };
        self.journal.book(layover).expect("books");
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn after(base: Timestamp, minutes: i64) -> Timestamp {
    base + SignedDuration::from_mins(minutes)
}

fn stamp(value: &Value) -> Timestamp {
    value.as_str().expect("a timestamp").parse().expect("valid")
}

async fn upcoming(dashboard: Dashboard, query: &str) -> Value {
    let response = router(dashboard, Guard::Open)
        .oneshot(
            Request::builder()
                .uri(format!("/upcoming{query}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

fn workflow<'a>(upcoming: &'a Value, name: &str) -> &'a Value {
    upcoming["workflows"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|w| w["pipeline"] == name)
        .unwrap_or_else(|| panic!("{name} is listed: {upcoming}"))
}

fn fires_of<'a>(upcoming: &'a Value, name: &str) -> Vec<&'a Value> {
    upcoming["fires"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|fire| fire["pipeline"] == name)
        .collect()
}

#[tokio::test]
async fn a_dashboard_with_no_tower_says_it_has_no_clock_rather_than_guessing() {
    // An `every` schedule is counted from when the Tower started, so a dashboard that worked the
    // times out for itself would show a timetable that looks exact and is not.
    let factory = Factory::new("watching", FACTORY);
    factory.set_down(after(Timestamp::now(), 60), Some("development"));

    let upcoming = upcoming(factory.dashboard(), "").await;

    assert!(upcoming["clock"].is_null(), "{upcoming}");
    assert_eq!(upcoming["fires"], serde_json::json!([]));
    let sweep = workflow(&upcoming, "sweep");
    assert!(sweep["next_at"].is_null(), "{sweep}");
    assert_eq!(sweep["fires_in_window"], 0);
    assert!(
        upcoming["workflows"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|w| w["pipeline"] != "development"),
        "a manual workflow fires only when somebody triggers it: {upcoming}"
    );

    let layover = &upcoming["layovers"][0];
    assert_eq!(layover["waiting_for"], "comments on pull request 41");
    assert_eq!(layover["pipeline"], "development");
    assert!(layover["collected_at"].is_null(), "{layover}");
}

#[tokio::test]
async fn each_schedule_lists_its_ticks_in_the_window_soonest_first() {
    let base = Timestamp::now();
    let factory = Factory::new("ticks", FACTORY);
    let clock = Fixed {
        next: vec![
            ("sweep", after(base, 10)),
            ("poll", after(base, 1)),
            ("follow_up", after(base, 5)),
        ],
        working: Vec::new(),
    };

    let upcoming = upcoming(factory.clocked(clock), "").await;

    assert_eq!(
        upcoming["clock"],
        "the Tower in `layover serve` (process 7)"
    );
    let sweep = fires_of(&upcoming, "sweep");
    assert_eq!(sweep.len(), 24, "10 past each hour for a day");
    assert_eq!(stamp(&sweep[0]["at"]), after(base, 10));
    assert_eq!(stamp(&sweep[23]["at"]), after(base, 23 * 60 + 10));
    assert_eq!(workflow(&upcoming, "sweep")["fires_in_window"], 24);
    assert_eq!(
        stamp(&workflow(&upcoming, "sweep")["next_at"]),
        after(base, 10)
    );

    // Counted whole, listed in part, so it does not bury the hourly one.
    assert_eq!(workflow(&upcoming, "poll")["fires_in_window"], 1440);
    assert_eq!(fires_of(&upcoming, "poll").len(), 24);

    let times: Vec<Timestamp> = upcoming["fires"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|fire| stamp(&fire["at"]))
        .collect();
    assert!(
        times.windows(2).all(|pair| pair[0] <= pair[1]),
        "soonest first: {times:?}"
    );
    assert!(times.iter().all(|at| *at <= stamp(&upcoming["until"])));
}

#[tokio::test]
async fn only_the_next_tick_of_a_workflow_still_working_may_be_skipped() {
    let base = Timestamp::now();
    let factory = Factory::new("working", FACTORY);
    let clock = Fixed {
        next: vec![("sweep", after(base, 10)), ("poll", after(base, 1))],
        working: vec!["sweep", "poll"],
    };

    let upcoming = upcoming(factory.clocked(clock), "").await;

    assert_eq!(workflow(&upcoming, "sweep")["working"], true);
    let sweep = fires_of(&upcoming, "sweep");
    assert_eq!(sweep[0]["may_skip"], true, "{}", sweep[0]);
    assert_eq!(
        sweep[1]["may_skip"], false,
        "what is running in an hour is not known now"
    );
    assert!(
        fires_of(&upcoming, "poll")
            .iter()
            .all(|fire| fire["may_skip"] == false),
        "`overlap = \"allow\"` starts whatever is running"
    );
}

#[tokio::test]
async fn a_layover_is_shown_with_the_resuming_tick_that_will_pick_it_up() {
    // Due at an hour from now, behind a 45-minute schedule next due in five minutes: picked up at
    // 5 + 45 + 45 minutes, not at the hour.
    let base = Timestamp::now();
    let factory = Factory::new("collect", FACTORY);
    factory.set_down(after(base, 60), Some("development"));
    factory.set_down(after(base, -30), None);
    let clock = Fixed {
        next: vec![("follow_up", after(base, 5))],
        working: Vec::new(),
    };

    let upcoming = upcoming(factory.clocked(clock), "").await;

    let layovers = upcoming["layovers"].as_array().expect("a list");
    assert_eq!(layovers.len(), 2);
    assert_eq!(
        stamp(&layovers[0]["collected_at"]),
        after(base, 5),
        "already due: the next tick takes it"
    );
    assert!(layovers[0]["pipeline"].is_null());
    assert_eq!(stamp(&layovers[1]["collected_at"]), after(base, 95));
    assert_eq!(layovers[1]["collected_by"], "follow_up");

    let ticks = fires_of(&upcoming, "follow_up");
    assert_eq!(ticks[0]["collects"], 1);
    assert_eq!(ticks[1]["collects"], 0);
    assert_eq!(ticks[2]["collects"], 1);
    assert_eq!(ticks[2]["resumes"], true);
}

#[tokio::test]
async fn a_layover_no_schedule_resumes_has_no_time_to_be_picked_up() {
    let base = Timestamp::now();
    let factory = Factory::new("uncollected", &FACTORY.replace("resumes = true\n", ""));
    factory.set_down(after(base, 60), None);
    let clock = Fixed {
        next: vec![("follow_up", after(base, 5))],
        working: Vec::new(),
    };

    let upcoming = upcoming(factory.clocked(clock), "").await;

    let layover = &upcoming["layovers"][0];
    assert!(layover["collected_at"].is_null(), "{layover}");
    assert!(layover["collected_by"].is_null(), "{layover}");
}

#[tokio::test]
async fn a_held_tick_is_overdue_and_the_ones_after_it_count_from_now() {
    // A Ground Stop holds the clock: the 07:00 tick fires the moment it is released, and the next
    // is an hour after that, not at 08:00.
    let base = Timestamp::now();
    let factory = Factory::new("held", FACTORY);
    fs::write(factory.root.join(".layover").join("ground-stop"), "").expect("engages");
    let clock = Fixed {
        next: vec![("sweep", after(base, -120))],
        working: Vec::new(),
    };

    let upcoming = upcoming(factory.clocked(clock), "").await;

    assert_eq!(upcoming["ground_stop"], true);
    let sweep = fires_of(&upcoming, "sweep");
    assert_eq!(sweep[0]["overdue"], true);
    assert_eq!(stamp(&sweep[0]["at"]), after(base, -120));
    assert_eq!(sweep[1]["overdue"], false);
    let second = stamp(&sweep[1]["at"]);
    assert!(
        second >= after(base, 60) && second < after(base, 61),
        "an hour from now: {second}"
    );
}

#[tokio::test]
async fn skipped_ticks_from_the_last_week_are_counted_and_listed() {
    let base = Timestamp::now();
    let factory = Factory::new("skips", FACTORY);
    for minutes in [-60, -180, -60 * 24 * 8] {
        factory
            .journal
            .record_skip(&Skip::still_working(
                PipelineName::new("sweep"),
                after(base, minutes),
            ))
            .expect("records");
    }

    let upcoming = upcoming(factory.dashboard(), "").await;

    let sweep = workflow(&upcoming, "sweep");
    assert_eq!(
        sweep["skipped_7d"], 2,
        "the one eight days ago is outside the week"
    );
    assert_eq!(stamp(&sweep["last_skipped_at"]), after(base, -60));
    assert_eq!(upcoming["skips"].as_array().map(Vec::len), Some(2));
    assert_eq!(upcoming["skips"][0]["reason"], "still_working");
    assert_eq!(workflow(&upcoming, "poll")["skipped_7d"], 0);
}

#[tokio::test]
async fn the_window_is_held_between_an_hour_and_a_week() {
    let factory = Factory::new("window", FACTORY);

    for (asked, hours) in [("?hours=1000", 168), ("?hours=0", 1), ("", 24)] {
        let upcoming = upcoming(factory.dashboard(), asked).await;
        let span = stamp(&upcoming["until"]).duration_since(stamp(&upcoming["now"]));
        assert_eq!(span, SignedDuration::from_hours(hours), "{asked}");
    }
}
