//! A Tower coming back settles every run the Tower before it never saw finish.
//!
//! Each test writes the live record a Tower that went away would have left, and points it at a real
//! process — one still running, or one that has exited — then opens the factory again.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::cost::CostSource;
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};
use layover_core::handover::{Interruption, Recovery};
use layover_core::queue::Queued;
use layover_core::run::Outcome;
use layover_tower::recovery::{Found, Settled};
use layover_tower::{Dispatched, Factory, Ledger, Live};

mod runs;
use runs::{Temp, factory, history, human, live_records, payloads, sleeps};

fn text(runner: &str) -> String {
    budgeted(runner, 100.0)
}

fn budgeted(runner: &str, fuel: f64) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
timeout_sec = 60
max_recovery_attempts = 2
fuel_usd = {fuel:?}

[runners.shell]
command = {runner}

[agents.eagle]
prompt = "review"
entry = true

[agents.mailman]
prompt = "post"
entry = true
recovery = "manual"

[agents.dev]
prompt = "develop"
entry = true

[agents.tester]
prompt = "test"

[agents.reviewer]
prompt = "review"

[[routes]]
from = "dev"
to = ["tester", "reviewer"]

[[routes]]
from = ["tester", "reviewer"]
to = "dev"
join = "all"
"#
    )
}

fn quick() -> &'static str {
    if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    }
}

/// A process that has come and gone.
fn exited() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/c", "exit 0"]).spawn()
    } else {
        Command::new("sh").args(["-c", "true"]).spawn()
    }
    .expect("spawns");
    let pid = child.id();
    let _ = child.wait();
    pid
}

/// A process that would run for a minute, and the thread that reaps it once it is stopped.
///
/// Reaped because a stopped child of this test process would otherwise linger as a zombie that
/// `ps` still lists; a real orphan's Tower is gone, and the system reaps it.
fn running() -> (u32, std::thread::JoinHandle<()>) {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 61 127.0.0.1 >nul"]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]);
        command
    };
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawns");
    let pid = child.id();
    (
        pid,
        std::thread::spawn(move || {
            let _ = child.wait();
        }),
    )
}

/// Writes the live record a Tower that went away leaves behind, and a transcript worth $15.11.
fn left_behind(temp: &Temp, pid: u32, agent: &str, queued: Option<Queued>) -> Live {
    let run = RunId::generate();
    let hangar = temp
        .0
        .join(".layover")
        .join("hangars")
        .join(agent)
        .join(run.as_str());
    std::fs::create_dir_all(&hangar).expect("hangar");
    std::fs::write(
        hangar.join("transcript.log"),
        "{\"type\":\"session.usage_checkpoint\",\"data\":{\"totalNanoAiu\":1510581560000}}\n",
    )
    .expect("transcript");

    let live = Live {
        run,
        itinerary: queued
            .as_ref()
            .map_or_else(ItineraryId::generate, |queued| {
                queued.flight.itinerary.clone()
            }),
        agent: AgentName::new(agent),
        pid,
        started_at: Timestamp::now(),
        hangar,
        queued,
        owner: Some("tower-that-went-away".to_owned()),
    };
    Ledger::open(temp.0.join(".layover").join("state").join("runs"))
        .expect("ledger")
        .starting(&live)
        .expect("records");
    live
}

fn work(agent: &str, body: &str) -> Queued {
    Queued::new(
        Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            AgentName::new(agent),
            body,
            6,
        ),
        None,
        BTreeMap::new(),
    )
}

fn reconcile(factory: &Factory) -> (Vec<Settled>, Vec<Queued>) {
    let mut queued = Vec::new();
    let settled = factory.reconcile(&mut |restart| {
        queued.push(restart);
        Ok(())
    });
    (settled, queued)
}

#[test]
fn a_run_still_alive_after_its_tower_went_away_is_stopped_recorded_and_restarted_once() {
    let temp = Temp::new("recover-alive");
    let (pid, reaper) = running();
    let asked = work("eagle", "review pull request 42");
    let old = left_behind(&temp, pid, "eagle", Some(asked.clone()));

    let tower = factory(&temp, &text(quick()));
    let (settled, restarts) = reconcile(&tower);
    reaper.join().expect("the orphan was stopped");

    assert_eq!(settled.len(), 1, "{settled:?}");
    assert_eq!(settled[0].found, Found::Stopped);
    assert_eq!(settled[0].restarted, Ok(2));
    assert!(live_records(&temp.0).is_empty(), "its live record is gone");

    let interrupted = history(&temp.0);
    assert_eq!(interrupted.len(), 1, "recorded, once");
    assert_eq!(interrupted[0].run, old.run);
    assert_eq!(interrupted[0].outcome, Outcome::Interrupted);
    assert_eq!(interrupted[0].source, CostSource::CopilotCredits);
    assert!((interrupted[0].usd - 15.105_815_6).abs() < 1e-9);
    assert!(
        interrupted[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("was stopped")),
        "{:?}",
        interrupted[0].detail
    );

    // The same work, in the same chain, told what it is restarting.
    assert_eq!(restarts.len(), 1);
    let restart = &restarts[0];
    assert_eq!(restart.flight.itinerary, asked.flight.itinerary);
    assert_eq!(restart.flight.to, asked.flight.to);
    assert_eq!(restart.flight.body, asked.flight.body);
    assert_ne!(restart.flight.id, asked.flight.id);
    assert_eq!(
        restart.recovering,
        Some(Recovery {
            previous: old.run.clone(),
            interruption: Interruption::TowerRestart,
            attempt: 2,
        })
    );

    // A second restart finds nothing left to settle, so the work is queued exactly once.
    let (again, more) = reconcile(&factory(&temp, &text(quick())));
    assert!(again.is_empty() && more.is_empty(), "{again:?} {more:?}");

    let mut ran = Vec::new();
    tower.drain(restarts, |_| {}, |_, result| ran.push(result.to_string()));
    assert_eq!(ran.len(), 1, "{ran:?}");
    let told = payloads(&temp.0, "eagle");
    let told = told.last().expect("the restart ran");
    assert!(
        told.contains("You are continuing interrupted work"),
        "{told}"
    );
    assert!(told.contains(old.run.as_str()), "{told}");
    assert!(told.contains("attempt 2"), "{told}");
    assert!(told.contains("review pull request 42"), "{told}");
}

#[test]
fn a_run_left_by_a_release_that_kept_no_work_is_still_recorded_and_forgotten() {
    // What 1.3.0 left behind: a record with no work and no owner. It used to stay there forever,
    // with the run missing from history.
    let temp = Temp::new("recover-legacy");
    let mut old = left_behind(&temp, exited(), "eagle", None);
    old.owner = None;
    Ledger::open(temp.0.join(".layover").join("state").join("runs"))
        .expect("ledger")
        .starting(&old)
        .expect("rewrites");

    let (settled, restarts) = reconcile(&factory(&temp, &text(quick())));

    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].found, Found::Exited);
    assert!(settled[0].restarted.is_err());
    assert!(restarts.is_empty());
    assert!(live_records(&temp.0).is_empty());
    let recorded = history(&temp.0);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, Outcome::Interrupted);
}

#[test]
fn a_run_another_living_tower_is_watching_is_left_alone() {
    // `layover run` beside a `serve`, or a Tower started before the last one has gone.
    let temp = Temp::new("recover-owned");
    let config = text(&sleeps(30));
    let first = factory(&temp, &config);
    let stop = temp.0.join(".layover").join("ground-stop");

    std::thread::scope(|scope| {
        let watching = scope.spawn(|| first.drain(vec![human("eagle")], |_| {}, |_, _| {}));
        let began = std::time::Instant::now();
        while live_records(&temp.0).is_empty() {
            assert!(began.elapsed().as_secs() < 20, "the run never started");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let (settled, restarts) = reconcile(&factory(&temp, &config));
        assert!(settled.is_empty(), "{settled:?}");
        assert!(restarts.is_empty());
        assert_eq!(live_records(&temp.0).len(), 1, "its record is untouched");

        std::fs::write(&stop, "").expect("engages");
        watching.join().expect("joins");
    });

    let runs = history(&temp.0);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].outcome, Outcome::Halted, "ended by its own Tower");
}

#[test]
fn an_agent_that_must_not_repeat_its_work_is_stopped_but_left_for_a_person() {
    let temp = Temp::new("recover-manual");
    let (pid, reaper) = running();
    left_behind(
        &temp,
        pid,
        "mailman",
        Some(work("mailman", "post the summary")),
    );

    let (settled, restarts) = reconcile(&factory(&temp, &text(quick())));
    reaper.join().expect("stopped");

    assert_eq!(settled[0].found, Found::Stopped);
    let why = settled[0].restarted.clone().expect_err("not restarted");
    assert!(why.contains("manual"), "{why}");
    assert!(restarts.is_empty());
}

#[test]
fn a_crash_loop_stops_at_the_attempt_limit() {
    let temp = Temp::new("recover-limit");
    let third = work("eagle", "again").restarting(Recovery {
        previous: RunId::generate(),
        interruption: Interruption::TowerRestart,
        attempt: 3,
    });
    left_behind(&temp, exited(), "eagle", Some(third));

    let (settled, restarts) = reconcile(&factory(&temp, &text(quick())));

    let why = settled[0].restarted.clone().expect_err("not restarted");
    assert!(why.contains("already restarted 2"), "{why}");
    assert!(restarts.is_empty());
}

#[test]
fn nothing_restarts_through_a_ground_stop() {
    let temp = Temp::new("recover-grounded");
    let (pid, reaper) = running();
    left_behind(&temp, pid, "eagle", Some(work("eagle", "review")));
    std::fs::create_dir_all(temp.0.join(".layover")).expect("dirs");
    std::fs::write(temp.0.join(".layover").join("ground-stop"), "").expect("engages");

    let (settled, restarts) = reconcile(&factory(&temp, &text(quick())));
    reaper.join().expect("stopped all the same");

    assert_eq!(settled[0].found, Found::Stopped);
    assert_eq!(
        settled[0].restarted,
        Err("a Ground Stop is engaged".to_owned())
    );
    assert!(restarts.is_empty());
}

#[test]
fn a_restarted_join_goes_straight_to_its_agent() {
    // Its barrier went with the Tower that held it. Delivered to a fresh one, it would wait for a
    // reviewer whose run finished before the restart, and be given up as unreachable.
    let temp = Temp::new("recover-join");
    let chain = ItineraryId::generate();
    let joined = Queued::new(
        Flight::new(
            chain,
            Origin::Agent(AgentName::new("tester")),
            AgentName::new("dev"),
            "tester: green\n\nreviewer: approved",
            5,
        ),
        None,
        BTreeMap::new(),
    )
    .already_released();
    left_behind(&temp, exited(), "dev", Some(joined));

    let tower = factory(&temp, &text(quick()));
    let (_, restarts) = reconcile(&tower);
    assert!(restarts[0].released);

    let mut results = Vec::new();
    let drained = tower.drain(
        restarts,
        |_| {},
        |_, result| {
            results.push((matches!(result, Dispatched::Ran { .. }), result.to_string()));
        },
    );

    assert!(
        matches!(results.as_slice(), [(true, _)]),
        "ran once, without parking: {results:?}"
    );
    assert!(drained.abandoned.is_empty());
    let told = payloads(&temp.0, "dev");
    assert!(
        told.last()
            .is_some_and(|told| told.contains("reviewer: approved"))
    );
}

#[test]
fn a_restart_is_admitted_against_what_the_run_it_replaces_spent() {
    // Recovery is a rail, not a reflex: the interrupted run's $15.11 is charged to its chain, so a
    // chain with $10 of Fuel cannot buy itself a fresh budget by being interrupted.
    let temp = Temp::new("recover-fuel");
    left_behind(&temp, exited(), "eagle", Some(work("eagle", "review")));

    let tower = factory(&temp, &budgeted(quick(), 10.0));
    let (_, restarts) = reconcile(&tower);
    assert_eq!(restarts.len(), 1, "the policy allows it");

    let mut results = Vec::new();
    tower.drain(
        restarts,
        |_| {},
        |_, result| results.push(result.to_string()),
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].contains("fuel exhausted"), "{results:?}");
}
