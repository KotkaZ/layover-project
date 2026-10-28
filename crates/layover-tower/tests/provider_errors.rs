//! A run the model service ended is restarted like an interrupted one; a run that failed its own
//! work is not.
//!
//! The stand-in runners print Copilot CLI's events as a real transcript has them and exit 1. One
//! always does; the other does so unless it is told it is attempt 2, so a restart of it succeeds.

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use layover_core::handover::Interruption;
use layover_core::itinerary::Denial;
use layover_core::queue::Queued;
use layover_core::run::{FailureCause, Outcome, RunRecord};
use layover_store::Journal;
use layover_tower::{Dispatched, Factory, Refusal};

mod runs;
use runs::{Temp, factory, history, human, payloads};

/// Work, then a model call the service rejected, then the bookkeeping before the CLI exits.
const REJECTED: &str = r#"{"type":"assistant.turn_start","data":{"turnId":"28"}}
{"type":"tool.execution_start","data":{"toolCallId":"t28","toolName":"powershell"}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","result":"success"}}
{"type":"tool.execution_complete","data":{"toolCallId":"t28","success":true,"result":{"content":"49/49 checks passed"}}}
{"type":"model.call_start","data":{"turnId":"29","model":"gpt-6.1-sol"}}
{"type":"model.call_finished","data":{"turnId":"29","outcome":"error","dispatchDurationMs":606}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","isByok":false,"result":"http_400"}}
{"type":"session.error","data":{"errorType":"query","message":"Execution failed: 400 internal server error (Request ID: 00000-7a12)","statusCode":400,"providerCallId":"00000-7a12","serviceRequestId":"9e19"}}
{"type":"session.usage_checkpoint","data":{"totalNanoAiu":1000000000}}
{"type":"assistant.idle","data":{}}
{"type":"result","sessionId":"753e","exitCode":1}
"#;

/// Work whose last tool failed: the run's own failure, quoting a provider error it read in a log.
const TESTS_FAILED: &str = r#"{"type":"tool.execution_start","data":{"toolCallId":"t28","toolName":"powershell"}}
{"type":"model.call_final_result","data":{"model":"gpt-6.1-sol","result":"success"}}
{"type":"tool.execution_complete","data":{"toolCallId":"t28","success":false,"result":{"content":"3 tests failed; the log said {\"type\":\"session.error\",\"data\":{\"errorType\":\"query\"}}"}}}
{"type":"result","sessionId":"753e","exitCode":1}
"#;

/// A run that never started its work: the CLI found the model it was given not available, and said
/// so in a line of plain text rather than an event.
const MODEL_GONE: &str = r#"{"type":"session.mcp_server_tools_loaded","data":{"serverName":"layover"}}
Error: Model "gpt-6.1-sol" from --model flag is not available.
"#;

/// Writes `stream` and a script that prints it and exits 1 — unless `recovers`, when a run told it
/// is attempt 2 exits 0 instead. Returns the runner's `command`.
fn stand_in(temp: &Temp, name: &str, stream: &str, recovers: bool) -> String {
    let dir = temp.0.join("stand-ins");
    std::fs::create_dir_all(&dir).expect("a directory");
    let events = dir.join(format!("{name}.jsonl"));
    std::fs::write(&events, stream).expect("writes the stream");

    if cfg!(windows) {
        let script = dir.join(format!("{name}.cmd"));
        let recover = if recovers {
            "findstr /c:\"This is attempt 2\" >nul && exit /b 0\r\n"
        } else {
            ""
        };
        std::fs::write(
            &script,
            format!(
                "@echo off\r\n{recover}type \"{}\"\r\nexit /b 1\r\n",
                events.display()
            ),
        )
        .expect("writes the script");
        format!(r#"["cmd", "/c", '{}']"#, script.display())
    } else {
        let script = dir.join(format!("{name}.sh"));
        let recover = if recovers {
            "if grep -q 'This is attempt 2'; then exit 0; fi\n"
        } else {
            ""
        };
        std::fs::write(
            &script,
            format!("{recover}cat '{}'\nexit 1\n", events.display()),
        )
        .expect("writes the script");
        format!(r#"["sh", '{}']"#, script.display())
    }
}

/// A script that prints the rejection and then outlives its timeout.
fn outlives(temp: &Temp) -> String {
    let dir = temp.0.join("stand-ins");
    std::fs::create_dir_all(&dir).expect("a directory");
    let events = dir.join("slow.jsonl");
    std::fs::write(&events, REJECTED).expect("writes the stream");
    if cfg!(windows) {
        let script = dir.join("slow.cmd");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\ntype \"{}\"\r\nping -n 6 127.0.0.1 >nul\r\nexit /b 1\r\n",
                events.display()
            ),
        )
        .expect("writes");
        format!(r#"["cmd", "/c", '{}']"#, script.display())
    } else {
        let script = dir.join("slow.sh");
        std::fs::write(
            &script,
            format!("cat '{}'\nsleep 5\nexit 1\n", events.display()),
        )
        .expect("writes");
        format!(r#"["sh", '{}']"#, script.display())
    }
}

fn text(fails: &str, recovers: &str, rails: &str) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "fails"
timeout_sec = 60
max_recovery_attempts = 2
{rails}

[runners.fails]
command = {fails}
provider_errors = {{ read = "copilot" }}

[runners.recovers]
command = {recovers}
provider_errors = {{ read = "copilot" }}

[agents.wolf]
prompt = "verify"
entry = true

[agents.bob]
prompt = "build"
entry = true
runner = "recovers"

[agents.mailman]
prompt = "post"
entry = true
recovery = "manual"

[agents.hermit]
prompt = "think"
entry = true
recovery = "never"
"#
    )
}

fn tower(temp: &Temp, rails: &str) -> Factory {
    let fails = stand_in(temp, "fails", REJECTED, false);
    let recovers = stand_in(temp, "recovers", REJECTED, true);
    factory(temp, &text(&fails, &recovers, rails))
}

fn journal(root: &Path) -> Journal {
    Journal::open(root.join(".layover").join("journal")).expect("opens")
}

/// Runs `work`, and what each flight came to.
fn run(tower: &Factory, work: Vec<Queued>) -> Vec<Dispatched> {
    let mut results = Vec::new();
    tower.drain(work, |_| {}, |_, result| results.push(result.clone()));
    results
}

fn only<'a>(records: &'a [RunRecord], agent: &str) -> Vec<&'a RunRecord> {
    records
        .iter()
        .filter(|record| record.agent.as_str() == agent)
        .collect()
}

#[test]
fn a_run_the_model_service_ended_is_recorded_as_its_failure_and_restarted_after_a_minute() {
    let temp = Temp::new("provider-restart");
    let tower = tower(&temp, "");
    let work = human("wolf");

    let results = run(&tower, vec![work.clone()]);

    assert!(
        matches!(&results[..], [Dispatched::Ran { outcome: Outcome::Failed, restart: Some(said), .. }]
            if said.contains("restarting as attempt 2 after a 1-minute wait")),
        "{results:?}"
    );

    let records = history(&temp.0);
    let record = &records[0];
    assert_eq!(
        record.outcome,
        Outcome::Failed,
        "never surfaced as succeeded"
    );
    assert_eq!(record.cause, Some(FailureCause::ProviderError));
    assert_eq!(record.request_id.as_deref(), Some("00000-7a12"));
    let detail = record.detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with("exited with code 1: the model service failed (http_400)"),
        "{detail}"
    );
    assert!(
        detail.ends_with("restarting as attempt 2 after a 1-minute wait"),
        "{detail}"
    );

    let pending = journal(&temp.0).pending().expect("reads the queue");
    let [restart] = &pending[..] else {
        panic!("one restart queued: {pending:?}");
    };
    assert_eq!(
        restart.flight.itinerary, work.flight.itinerary,
        "the same chain"
    );
    assert_eq!(restart.flight.to, work.flight.to);
    assert_eq!(restart.flight.body, work.flight.body, "the same work");
    assert_ne!(restart.flight.id, work.flight.id, "a new flight for it");
    assert_eq!(restart.attempt(), 2);
    let recovering = restart.recovering.as_ref().expect("a restart");
    assert_eq!(recovering.previous, record.run);
    assert_eq!(
        recovering.interruption,
        Interruption::ProviderError {
            request_id: Some("00000-7a12".to_owned())
        }
    );
    let wait = restart
        .not_before
        .expect("backs off")
        .duration_since(Timestamp::now());
    assert!(
        wait > SignedDuration::from_secs(50) && wait <= SignedDuration::from_secs(60),
        "{wait:?}"
    );
}

/// The queued restart, due now.
fn due_restart(root: &Path) -> Queued {
    let journal = journal(root);
    let pending = journal.pending().expect("reads the queue");
    let restart = pending.last().expect("a restart is queued").clone();
    journal.unqueue(&restart.flight.id).expect("takes it off");
    Queued {
        not_before: Some(Timestamp::now()),
        ..restart
    }
}

#[test]
fn attempts_are_counted_per_flight_so_an_outage_cannot_loop() {
    let temp = Temp::new("provider-attempts");
    let tower = tower(&temp, "");

    run(&tower, vec![human("wolf")]);
    let second = run(&tower, vec![due_restart(&temp.0)]);
    assert!(
        matches!(&second[..], [Dispatched::Ran { restart: Some(said), .. }]
            if said.contains("attempt 3 after a 5-minute wait")),
        "a longer wait before the second restart: {second:?}"
    );

    let third = run(&tower, vec![due_restart(&temp.0)]);
    assert!(
        matches!(&third[..], [Dispatched::Ran { restart: Some(said), .. }]
            if said.contains("not restarted") && said.contains("already restarted 2 time(s)")),
        "{third:?}"
    );
    assert!(journal(&temp.0).pending().expect("reads").is_empty());

    let told = payloads(&temp.0, "wolf");
    assert_eq!(told.len(), 3);
    assert!(
        told[1].contains("the model service failed mid-run (request 00000-7a12)"),
        "{}",
        told[1]
    );
    assert!(told[1].contains("This is attempt 2"));
    assert!(told[1].contains("It was not the work that failed"));
    assert!(told[2].contains("This is attempt 3"));
    assert!(!told[0].contains("It was not the work that failed"));
}

#[test]
fn a_restart_that_succeeds_ends_the_chain_as_its_work_did() {
    let temp = Temp::new("provider-recovers");
    let tower = tower(&temp, "");

    run(&tower, vec![human("bob")]);
    let results = run(&tower, vec![due_restart(&temp.0)]);

    assert!(
        matches!(
            &results[..],
            [Dispatched::Ran {
                outcome: Outcome::Succeeded,
                restart: None,
                ..
            }]
        ),
        "{results:?}"
    );
    let runs = history(&temp.0);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].itinerary, runs[1].itinerary, "one chain");
    assert_eq!(runs[1].cause, None);
}

#[test]
fn an_agent_that_must_not_repeat_its_work_waits_for_a_person() {
    let temp = Temp::new("provider-policy");
    let tower = tower(&temp, "");

    run(&tower, vec![human("mailman"), human("hermit")]);

    let records = history(&temp.0);
    for (agent, policy) in [("mailman", "manual"), ("hermit", "never")] {
        let record = only(&records, agent)[0];
        assert_eq!(record.cause, Some(FailureCause::ProviderError), "{agent}");
        let detail = record.detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains(&format!(
                "not restarted: this agent's `recovery` policy is `{policy}`"
            )),
            "{detail}"
        );
    }
    assert!(journal(&temp.0).pending().expect("reads").is_empty());
}

#[test]
fn turning_recovery_off_for_the_factory_turns_it_off_for_provider_errors() {
    let temp = Temp::new("provider-off");
    let text = text(
        &stand_in(&temp, "fails", REJECTED, false),
        &stand_in(&temp, "recovers", REJECTED, true),
        "",
    )
    .replace("max_recovery_attempts = 2", "max_recovery_attempts = 0");
    let tower = factory(&temp, &text);

    run(&tower, vec![human("wolf")]);

    assert!(journal(&temp.0).pending().expect("reads").is_empty());
    assert!(
        history(&temp.0)[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("already restarted 0 time(s)")),
        "{:?}",
        history(&temp.0)[0].detail
    );
}

#[test]
fn a_run_that_failed_its_own_work_is_never_restarted() {
    let temp = Temp::new("provider-own");
    let fails = stand_in(&temp, "own", TESTS_FAILED, false);
    let tower = factory(&temp, &text(&fails, &fails, ""));

    let results = run(&tower, vec![human("wolf")]);

    assert!(
        matches!(
            &results[..],
            [Dispatched::Ran {
                outcome: Outcome::Failed,
                restart: None,
                ..
            }]
        ),
        "{results:?}"
    );
    let record = &history(&temp.0)[0];
    assert_eq!(record.cause, None);
    assert_eq!(record.request_id, None);
    assert!(journal(&temp.0).pending().expect("reads").is_empty());
}

#[test]
fn a_run_that_ran_out_of_time_is_timed_out_not_a_provider_error() {
    let temp = Temp::new("provider-timeout");
    let slow = outlives(&temp);
    let tower = factory(
        &temp,
        &text(&slow, &slow, "").replace("timeout_sec = 60", "timeout_sec = 1"),
    );

    run(&tower, vec![human("wolf")]);

    let record = &history(&temp.0)[0];
    assert_eq!(record.outcome, Outcome::TimedOut);
    assert_eq!(record.cause, None);
    assert!(journal(&temp.0).pending().expect("reads").is_empty());
}

#[test]
fn a_restart_is_an_ordinary_run_that_the_chains_rails_still_bound() {
    // One run per chain: the run that failed spent it, so its restart is refused at admission.
    let temp = Temp::new("provider-rails");
    let tower = tower(&temp, "max_runs = 1");

    run(&tower, vec![human("wolf")]);
    let results = run(&tower, vec![due_restart(&temp.0)]);

    assert!(
        matches!(
            &results[..],
            [Dispatched::Refused(Refusal::Rail(Denial::RunCapReached))]
        ),
        "{results:?}"
    );
}

#[test]
fn a_restart_backing_off_is_waited_for_and_then_started() {
    // The queue a Tower drains: what it was handed, and whatever is queued since.
    let temp = Temp::new("provider-wait");
    let tower = tower(&temp, "");
    run(&tower, vec![human("bob")]);
    let soon = Queued {
        not_before: Some(Timestamp::now() + SignedDuration::from_millis(1_500)),
        ..due_restart(&temp.0)
    };
    let started = std::time::Instant::now();

    let mut results = Vec::new();
    let queue = journal(&temp.0);
    tower.drain_with(
        vec![soon],
        &mut |_| {},
        &mut |_, result| results.push(result.clone()),
        |_| queue.pending().unwrap_or_default(),
    );

    assert!(
        matches!(
            &results[..],
            [Dispatched::Ran {
                outcome: Outcome::Succeeded,
                ..
            }]
        ),
        "{results:?}"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(1_000),
        "not before it was due"
    );
}

#[test]
fn a_model_not_available_at_start_up_is_restarted_and_says_so_in_the_cli_s_words() {
    // The run woken by the work a chain had pending died in a short outage before it made a call,
    // and the chain sat silent for seventeen hours: the line was text, not an event, so it read as
    // the run's own failure.
    let temp = Temp::new("provider-model-gone");
    let gone = stand_in(&temp, "gone", MODEL_GONE, false);
    let tower = factory(&temp, &text(&gone, &gone, ""));

    run(&tower, vec![human("wolf"), human("mailman")]);

    let records = history(&temp.0);
    let said =
        r#"the model service failed: Model "gpt-6.1-sol" from --model flag is not available."#;
    let wolf = only(&records, "wolf")[0];
    assert_eq!(wolf.cause, Some(FailureCause::ProviderError));
    let detail = wolf.detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with(&format!(
            "exited with code 1: {said}; restarting as attempt 2"
        )),
        "{detail}"
    );
    let pending = journal(&temp.0).pending().expect("reads the queue");
    let mut restarted: Vec<&str> = pending
        .iter()
        .map(|restart| restart.flight.to.as_str())
        .collect();
    restarted.sort_unstable();
    assert_eq!(
        restarted,
        ["mailman", "wolf"],
        "both: the `manual` one too, since a run that never made a call did nothing to repeat"
    );

    let mailman = only(&records, "mailman")[0];
    assert_eq!(mailman.cause, Some(FailureCause::ProviderError));
    let detail = mailman.detail.as_deref().unwrap_or_default();
    assert!(detail.contains(said), "{detail}");
    assert!(
        detail.contains(
            "it started no tool, so restarting although this agent's `recovery` policy is `manual`"
        ),
        "{detail}"
    );
}
