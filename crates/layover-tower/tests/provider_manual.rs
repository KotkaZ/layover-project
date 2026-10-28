//! A `manual` agent's run the model service ended. Restarted when it had started no tool, because
//! there is nothing it could have done twice; otherwise left as `manual` says, and a person asked
//! through a help request, because a failed run in history is not where anybody looks.
//!
//! The Ground Stop case is in the Tower's own unit test of `restart_after`: a stand-in that engaged
//! one as it exited would race the wait loop, which ends a run it sees a Ground Stop under.

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use layover_core::cost::Window;
use layover_core::flight::{ItineraryId, Origin};
use layover_core::handover::Handover;
use layover_core::help::{Blocker, HelpRequest};
use layover_core::layover::{Layover, Standing};
use layover_core::queue::Queued;
use layover_core::run::{FailureCause, Outcome};
use layover_store::{HelpFilter, Journal};
use layover_tower::{Dispatched, Factory};

mod runs;
use runs::{Temp, factory, history, human, to};

/// Start-up, then the first call refused: 28 October's 401, before the run did anything.
const NOTHING_DONE: &str = r#"{"type":"session.mcp_server_tools_loaded","data":{"serverName":"layover"}}
{"type":"assistant.turn_start","data":{"turnId":"1"}}
{"type":"model.call_start","data":{"turnId":"1","model":"claude-opus-5.5"}}
{"type":"model.call_final_result","data":{"model":"claude-opus-5.5","isByok":false,"result":"http_401"}}
{"type":"session.error","data":{"errorType":"query","message":"Authorization error. Your credentials may be expired or invalid.","statusCode":401,"providerCallId":"00000-401a"}}
{"type":"result","sessionId":"a401","exitCode":1}
"#;

/// One tool started — it may have acted — then the same refusal.
const ONE_TOOL: &str = r#"{"type":"session.mcp_server_tools_loaded","data":{"serverName":"layover"}}
{"type":"tool.execution_start","data":{"toolCallId":"t1","toolName":"ado-create_pull_request_thread"}}
{"type":"model.call_final_result","data":{"model":"claude-opus-5.5","result":"success"}}
{"type":"tool.execution_complete","data":{"toolCallId":"t1","success":true,"result":{"content":"posted"}}}
{"type":"model.call_final_result","data":{"model":"claude-opus-5.5","isByok":false,"result":"http_401"}}
{"type":"session.error","data":{"errorType":"query","message":"Authorization error. Your credentials may be expired or invalid.","statusCode":401,"providerCallId":"00000-401b"}}
{"type":"result","sessionId":"b401","exitCode":1}
"#;

/// A runner whose process prints `stream` and exits 1, read as Copilot CLI.
fn tower(temp: &Temp, stream: &str, attempts: u32) -> Factory {
    let dir = temp.0.join("stand-ins");
    std::fs::create_dir_all(&dir).expect("a directory");
    let events = dir.join("stream.jsonl");
    std::fs::write(&events, stream).expect("writes the stream");
    let command = if cfg!(windows) {
        let script = dir.join("fails.cmd");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\ntype \"{}\"\r\nexit /b 1\r\n",
                events.display()
            ),
        )
        .expect("writes the script");
        format!(r#"["cmd", "/c", '{}']"#, script.display())
    } else {
        let script = dir.join("fails.sh");
        std::fs::write(&script, format!("cat '{}'\nexit 1\n", events.display()))
            .expect("writes the script");
        format!(r#"["sh", '{}']"#, script.display())
    };

    factory(
        temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "fails"
timeout_sec = 60
max_recovery_attempts = {attempts}

[runners.fails]
command = {command}
provider_errors = {{ read = "copilot" }}

[agents.mailman]
prompt = "post"
entry = true
recovery = "manual"
"#
        ),
    )
}

fn journal(root: &Path) -> Journal {
    Journal::open(root.join(".layover").join("journal")).expect("opens")
}

fn run(tower: &Factory, work: Vec<Queued>) -> Vec<Dispatched> {
    let mut results = Vec::new();
    tower.drain(work, |_| {}, |_, result| results.push(result.clone()));
    results
}

fn asked(root: &Path) -> Vec<HelpRequest> {
    let span = Window::AllTime.resolve(
        &(Timestamp::now() + SignedDuration::from_mins(1)).to_zoned(jiff::tz::TimeZone::UTC),
    );
    journal(root)
        .help(&span, &HelpFilter::default())
        .expect("reads help")
}

/// The queued restart, due now.
fn due_restart(root: &Path) -> Queued {
    let journal = journal(root);
    let restart = journal
        .pending()
        .expect("reads the queue")
        .last()
        .expect("a restart is queued")
        .clone();
    journal.unqueue(&restart.flight.id).expect("takes it off");
    Queued {
        not_before: Some(Timestamp::now()),
        ..restart
    }
}

#[test]
fn a_manual_agent_s_run_that_started_no_tool_is_restarted_and_says_why() {
    let temp = Temp::new("manual-untouched");
    let tower = tower(&temp, NOTHING_DONE, 2);
    let work = human("mailman");

    let results = run(&tower, vec![work.clone()]);

    let why = "it started no tool, so restarting although this agent's `recovery` policy is \
               `manual`, as attempt 2 after a 1-minute wait";
    assert!(
        matches!(&results[..], [Dispatched::Ran { outcome: Outcome::Failed, restart: Some(said), .. }]
            if said.ends_with(why)),
        "{results:?}"
    );
    let record = &history(&temp.0)[0];
    assert_eq!(record.cause, Some(FailureCause::ProviderError));
    let detail = record.detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with("exited with code 1: the model service failed (http_401)"),
        "{detail}"
    );
    assert!(detail.ends_with(why), "{detail}");

    let pending = journal(&temp.0).pending().expect("reads the queue");
    let [restart] = &pending[..] else {
        panic!("one restart queued: {pending:?}");
    };
    assert_eq!(restart.flight.itinerary, work.flight.itinerary);
    assert_eq!(restart.flight.to.as_str(), "mailman");
    assert_eq!(restart.attempt(), 2);
    assert!(restart.not_before.is_some(), "it backs off like any other");
    assert!(asked(&temp.0).is_empty(), "nobody needs asking");
}

#[test]
fn a_manual_agent_s_run_that_started_a_tool_is_left_and_a_person_is_asked() {
    let temp = Temp::new("manual-acted");
    let tower = tower(&temp, ONE_TOOL, 2);
    let work = human("mailman");

    run(&tower, vec![work.clone()]);

    let record = &history(&temp.0)[0];
    let detail = record.detail.as_deref().unwrap_or_default();
    assert!(
        detail.ends_with(
            "not restarted: this agent's `recovery` policy is `manual`; a human decides"
        ),
        "{detail}"
    );
    assert!(journal(&temp.0).pending().expect("reads").is_empty());

    let requests = asked(&temp.0);
    let [request] = &requests[..] else {
        panic!("one help request: {requests:?}");
    };
    assert_eq!(request.agent.as_str(), "mailman");
    assert_eq!(request.run, record.run);
    assert_eq!(request.itinerary, work.flight.itinerary);
    assert_eq!(request.blocker, Blocker::Environment);
    assert!(
        !request.fatal,
        "the work stopped, but nothing is waiting on an answer"
    );
    assert!(request.is_open());
    assert!(
        request.records_its_chain(),
        "so a reply continues it in its workflow"
    );
    assert!(
        request.summary.contains("not restarted") && request.summary.contains("`manual`"),
        "{}",
        request.summary
    );
    for said in [
        record.run.as_str(),
        work.flight.itinerary.as_str(),
        "00000-401b",
        "it had started 1 tool",
    ] {
        assert!(request.detail.contains(said), "{said}: {}", request.detail);
    }
}

#[test]
fn an_untouched_run_out_of_attempts_is_left_and_a_person_is_asked() {
    let temp = Temp::new("manual-attempts");
    let tower = tower(&temp, NOTHING_DONE, 1);

    run(&tower, vec![human("mailman")]);
    let second = run(&tower, vec![due_restart(&temp.0)]);

    assert!(
        matches!(&second[..], [Dispatched::Ran { restart: Some(said), .. }]
            if said.contains("not restarted: already restarted 1 time(s)")),
        "{second:?}"
    );
    assert!(journal(&temp.0).pending().expect("reads").is_empty());
    let requests = asked(&temp.0);
    let [request] = &requests[..] else {
        panic!("one help request, for the attempt that was left: {requests:?}");
    };
    assert_eq!(request.run, history(&temp.0)[1].run);
    assert!(
        request.detail.contains("it had started no tool"),
        "{}",
        request.detail
    );
}

#[test]
fn a_run_that_collected_a_layover_says_the_layover_is_gone() {
    // The case that loses work without a trace: the run that picked a pull request's follow-up up
    // was the only thing that knew about it, and nothing books it again.
    let temp = Temp::new("manual-layover");
    let tower = tower(&temp, ONE_TOOL, 2);
    let mut layover = Layover::book(
        "mailman".into(),
        ItineraryId::generate(),
        "new comments on pull request 1603439",
        Handover::dispatch(Vec::new()),
        Timestamp::now(),
        Timestamp::now(),
    );
    layover.standing = Standing::Resumed;
    let id = layover.id.clone();
    journal(&temp.0).book(layover).expect("books");

    run(
        &tower,
        vec![to(&ItineraryId::generate(), Origin::Resumed(id), "mailman")],
    );

    let requests = asked(&temp.0);
    let [request] = &requests[..] else {
        panic!("one help request: {requests:?}");
    };
    assert!(
        request.detail.contains(
            "It had collected the layover booked for \"new comments on pull request 1603439\""
        ),
        "{}",
        request.detail
    );
}
