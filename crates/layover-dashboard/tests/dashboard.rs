//! What the dashboard serves, end to end.
//!
//! Against a real configuration file and a real history directory, because the whole point of the
//! dashboard is that it reads both from disk on every request. A test with the config in memory
//! would not catch the thing most worth catching: that editing `layover.toml` and reloading
//! actually changes what you see.

use std::fs;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

/// A temporary factory: a configuration file and a history directory that clean themselves up.
struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after 1970")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        let factory = Self(root);
        factory.write_config(TWO_AGENTS);
        factory
    }

    fn write_config(&self, body: &str) {
        fs::write(self.0.join("layover.toml"), body).expect("writes config");
    }

    /// Writes a run into the segment for the day its own `started_at` names.
    ///
    /// Derived rather than fixed. History is one file per UTC day and is read by opening the files
    /// a span covers, so a record whose timestamp and filename disagree is simply never found —
    /// and a test that hard-codes the filename while stamping the record `now()` passes until the
    /// calendar moves past the window, then fails for reasons that look nothing like the cause.
    fn write_run(&self, line: &str) {
        let started_at = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| value["started_at"].as_str().map(ToOwned::to_owned))
            .and_then(|text| text.parse::<jiff::Timestamp>().ok())
            .expect("a run record with a parseable `started_at`");

        let day = started_at
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
            .to_string();

        let path = self.0.join("history").join(format!("runs-{day}.jsonl"));
        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    /// Writes a run `hours_ago`, into the segment for the day it actually happened.
    ///
    /// History is stored one file per UTC day and read by picking the files a span covers, so a
    /// run's timestamp and the file it lives in have to agree or it is simply not found.
    fn write_run_ago(&self, run: &str, hours_ago: i64, usd: f64) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(hours_ago))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();

        let path = self.0.join("history").join(format!("runs-{day}.jsonl"));
        let line = format!(
            r#"{{"run":"{run}","itinerary":"itn_r","agent":"analyst","outcome":"succeeded","started_at":"{at}","finished_at":"{at}","usd":{usd},"source":"reported"}}"#
        );

        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(&line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    /// Writes a run of `pipeline`, in an itinerary named after it, an hour ago.
    fn write_run_in(&self, pipeline: &str, run: &str, usd: f64) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(1))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let path = self.0.join("history").join(format!("runs-{day}.jsonl"));

        let line = format!(
            r#"{{"run":"{run}","itinerary":"itn_{pipeline}","agent":"analyst","pipeline":"{pipeline}","outcome":"succeeded","started_at":"{at}","finished_at":"{at}","usd":{usd},"source":"reported"}}"#
        );

        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(&line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    /// Raises an open help request against `itinerary`, an hour ago.
    fn write_help(&self, itinerary: &str, agent: &str, summary: &str) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(1))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let dir = self.0.join("journal");
        fs::create_dir_all(&dir).expect("journal dir");
        let path = dir.join(format!("help-{day}.jsonl"));

        let line = format!(
            r#"{{"agent":"{agent}","run":"run_h","itinerary":"{itinerary}","blocker":"access","summary":"{summary}","detail":"d","fatal":true,"at":"{at}"}}"#
        );

        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(&line);
        existing.push('\n');
        fs::write(path, existing).expect("writes help");
    }

    fn router(&self) -> Router {
        router(Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(self.0.join("history")).expect("opens history"),
            journal: std::sync::Arc::new(
                Journal::open(self.0.join("journal")).expect("opens journal"),
            ),
            ground_stop: self.0.join("ground-stop"),
        }))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const TWO_AGENTS: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"

[agents.developer]
prompt = "develop"

[pipelines.triage]
entry = "analyst"

[pipelines.triage.flags]
deep = { default = false, description = "Investigate before writing anything." }

[[routes]]
from = "analyst"
to = "developer"
"#;

/// Issues a request with a method other than GET, and reads the whole body.
async fn verb(app: Router, method: &str, path: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn call(app: Router, path: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Sends a JSON body, and reads the whole response.
async fn body(app: Router, method: &str, path: &str, payload: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(payload.to_owned()))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("response is valid json")
}

#[tokio::test]
async fn the_page_and_its_assets_are_served_from_the_binary() {
    // Embedded rather than read from disk: the binary is installed on its own, so a dashboard
    // that needed a directory of files beside it would only work from a source checkout.
    let factory = Factory::new("assets");

    for (path, needle) in [
        ("/", "<title>Layover</title>"),
        ("/style.css", ".routemap"),
        ("/app.js", "async function loadCost"),
    ] {
        let (status, body) = call(factory.router(), path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains(needle), "{path} is missing {needle}");
    }
}

#[tokio::test]
async fn the_route_map_is_drawn_from_the_file_on_disk() {
    let factory = Factory::new("graph");

    let (status, body) = call(factory.router(), "/graph").await;

    assert_eq!(status, StatusCode::OK);
    let value = json(&body);
    let svg = value["mermaid"].as_str().expect("svg source");
    assert!(svg.starts_with("<svg"), "{svg}");
    assert!(svg.contains(r#"id="a_analyst""#));
    assert!(svg.contains(r#"id="p_triage""#));
}

#[tokio::test]
async fn editing_the_configuration_changes_the_diagram_without_a_restart() {
    // This is the difference between a dashboard and a snapshot, and the reason the config is
    // re-read per request rather than cached at startup.
    let factory = Factory::new("reload");
    let (_, before) = call(factory.router(), "/graph").await;
    assert!(
        !json(&before)["mermaid"]
            .as_str()
            .unwrap()
            .contains("a_auditor")
    );

    factory.write_config(&format!(
        "{TWO_AGENTS}\n[agents.auditor]\nprompt = \"audit\"\n"
    ));

    let (_, after) = call(factory.router(), "/graph").await;
    assert!(
        json(&after)["mermaid"]
            .as_str()
            .unwrap()
            .contains("a_auditor"),
        "the new agent should appear on a plain reload"
    );
}

#[tokio::test]
async fn a_broken_configuration_is_reported_rather_than_served_as_an_empty_graph() {
    // An empty diagram looks like a factory with nothing in it, which is a very misleading way
    // to be told the file has a syntax error.
    let factory = Factory::new("broken");
    factory.write_config("this is not toml {{{");

    let (status, body) = call(factory.router(), "/graph").await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        json(&body)["title"]
            .as_str()
            .unwrap()
            .contains("could not be read")
    );
}

#[tokio::test]
async fn history_reaches_the_runs_endpoint() {
    let factory = Factory::new("runs");
    factory.write_run(
        r#"{"run":"run_a","itinerary":"itn_1","agent":"developer","pipeline":"triage","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:02:00Z","usd":1.5,"source":"reported"}"#,
    );

    let (status, body) = call(factory.router(), "/runs?window=all_time").await;

    assert_eq!(status, StatusCode::OK);
    let runs = json(&body)["runs"].as_array().expect("runs").clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["agent"], "developer");
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(runs[0]["duration_sec"], 120);
    assert_eq!(runs[0]["cost_usd"], 1.5);
}

#[tokio::test]
async fn an_unreported_cost_is_null_rather_than_zero() {
    // A zero is indistinguishable from a run that genuinely cost nothing, and the difference is
    // what decides whether the budget rail is working at all.
    let factory = Factory::new("unreported");
    factory.write_run(
        r#"{"run":"run_b","itinerary":"itn_1","agent":"analyst","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","source":"unreported"}"#,
    );

    let (_, body) = call(factory.router(), "/runs?window=all_time").await;

    assert!(json(&body)["runs"][0]["cost_usd"].is_null());
    assert_eq!(json(&body)["runs"][0]["cost_source"], "unreported");
}

#[tokio::test]
async fn a_calendar_window_names_the_zone_it_was_reckoned_in_and_a_rolling_one_does_not() {
    // The absence is the point. Nobody should have to wonder which zone "last 7 days" used, and
    // everybody should be told which one "month to date" used.
    let factory = Factory::new("zone");

    let (_, calendar) = call(factory.router(), "/costs?window=month_to_date").await;
    assert_eq!(json(&calendar)["span"]["calendar"], true);
    assert!(json(&calendar)["span"]["zone"].is_string());

    let (_, rolling) = call(factory.router(), "/costs?window=last_7d").await;
    assert_eq!(json(&rolling)["span"]["calendar"], false);
    assert!(json(&rolling)["span"]["zone"].is_null());
}

#[tokio::test]
async fn one_unreported_run_downgrades_the_whole_total() {
    let factory = Factory::new("confidence");
    factory.write_run(
        r#"{"run":"run_c","itinerary":"itn_1","agent":"analyst","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","usd":4.0,"source":"reported"}"#,
    );
    factory.write_run(
        r#"{"run":"run_d","itinerary":"itn_1","agent":"developer","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","source":"unreported"}"#,
    );

    let (_, body) = call(factory.router(), "/costs?window=all_time").await;
    let total = &json(&body)["total"];

    assert_eq!(total["runs"], 2);
    assert_eq!(total["usd"], 4.0);
    assert_eq!(total["confidence"], "unreported");
    assert_eq!(total["measured_share"], 0.5);
}

#[tokio::test]
async fn a_run_still_going_is_listed_but_not_billed() {
    let factory = Factory::new("live");
    factory.write_run(
        r#"{"run":"run_e","itinerary":"itn_1","agent":"developer","outcome":"running","started_at":"2026-09-16T10:00:00Z","source":"unreported"}"#,
    );

    let (_, runs) = call(factory.router(), "/runs?window=all_time").await;
    assert_eq!(json(&runs)["runs"].as_array().map(Vec::len), Some(1));
    assert!(json(&runs)["runs"][0]["finished_at"].is_null());

    let (_, costs) = call(factory.router(), "/costs?window=all_time").await;
    assert_eq!(json(&costs)["total"]["runs"], 0);
}

#[tokio::test]
async fn control_operations_refuse_rather_than_pretend() {
    // A control that silently does nothing is worse than one that is not there, because it gets
    // trusted once and then relied upon. Streaming a run is the one still unbuilt.
    let factory = Factory::new("control");

    let response = factory
        .router()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/runs/run_x/stream")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn a_ground_stop_can_be_engaged_and_released_over_http() {
    // The factory now runs unattended, so the kill switch has to be reachable from the page
    // somebody is watching it on — not only by creating a file by hand.
    let factory = Factory::new("groundstop");

    let (status, body) = verb(factory.router(), "POST", "/ground-stop").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"engaged\":true"), "{body}");
    assert!(
        factory.path().join("ground-stop").exists(),
        "it has to be on disk: the Tower reads it from there, and it must survive a crash"
    );

    let (status, body) = verb(factory.router(), "DELETE", "/ground-stop").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"engaged\":false"), "{body}");
    assert!(!factory.path().join("ground-stop").exists());
}

#[tokio::test]
async fn engaging_a_ground_stop_twice_is_not_an_error() {
    // Somebody pressing the button again because the first press was not obviously acknowledged
    // must not be told it failed. That is how people learn a kill switch is unreliable.
    let factory = Factory::new("groundstop-twice");

    let (first, _) = verb(factory.router(), "POST", "/ground-stop").await;
    let (second, body) = verb(factory.router(), "POST", "/ground-stop").await;

    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::OK, "{body}");
    assert!(body.contains("\"engaged\":true"), "{body}");
}

#[tokio::test]
async fn releasing_a_ground_stop_that_is_not_engaged_is_not_an_error() {
    let factory = Factory::new("groundstop-absent");

    let (status, body) = verb(factory.router(), "DELETE", "/ground-stop").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"engaged\":false"), "{body}");
}

#[tokio::test]
async fn resolving_help_clears_it_from_the_open_list() {
    // Resolving says the blocker is gone, not that somebody read it. An agent that hits the same
    // wall next run raises it again, which is what makes the list evidence of anything.
    let factory = Factory::new("resolve-help");
    factory.write_help("itn_1", "analyst", "the token expired");

    let (_, before) = call(factory.router(), "/help?open=true&window=last_7d").await;
    assert_eq!(json(&before)["requests"].as_array().map(Vec::len), Some(1));

    let (status, resolved) = body(
        factory.router(),
        "POST",
        "/help/resolve",
        r#"{"run_id":"run_h","window":"last_7d"}"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(json(&resolved)["resolved"], 1, "{resolved}");

    let (_, after) = call(factory.router(), "/help?open=true&window=last_7d").await;
    assert_eq!(json(&after)["requests"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn resolving_narrows_to_the_run_it_names() {
    // The list has a button per row, so the narrowest form has to actually narrow.
    let factory = Factory::new("resolve-narrow");
    factory.write_help("itn_1", "analyst", "the token expired");

    let (_, resolved) = body(
        factory.router(),
        "POST",
        "/help/resolve",
        r#"{"run_id":"run_somebody_else","window":"last_7d"}"#,
    )
    .await;

    assert_eq!(json(&resolved)["resolved"], 0, "{resolved}");

    let (_, after) = call(factory.router(), "/help?open=true&window=last_7d").await;
    assert_eq!(
        json(&after)["requests"].as_array().map(Vec::len),
        Some(1),
        "another run's request must be left alone"
    );
}

#[tokio::test]
async fn resolving_nothing_is_a_success_not_an_error() {
    // Somebody else may have resolved it a second earlier. That is not a failure.
    let factory = Factory::new("resolve-empty");

    let (status, resolved) = body(
        factory.router(),
        "POST",
        "/help/resolve",
        r#"{"window":"last_7d"}"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(json(&resolved)["resolved"], 0);
}

#[tokio::test]
async fn judging_a_learning_that_does_not_exist_is_a_404() {
    let factory = Factory::new("judge-missing");

    let (status, problem) = body(
        factory.router(),
        "PATCH",
        "/learnings/lrn_nope",
        r#"{"state":"confirmed"}"#,
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
    assert!(problem.contains("lrn_nope"), "{problem}");
}

#[tokio::test]
async fn cancelling_work_that_is_not_queued_says_so_rather_than_claiming_success() {
    // "Cancelled" about a run that is already going is the most dangerous thing this surface
    // could say: somebody would walk away from something still opening pull requests.
    let factory = Factory::new("cancel-missing");

    let (status, body) = verb(factory.router(), "DELETE", "/flights/flt_nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("flt_nope"), "{body}");
    assert!(
        body.contains("Ground Stop"),
        "it should say what does stop a live run: {body}"
    );
}

#[tokio::test]
async fn an_absent_run_is_a_404_with_the_identifier_in_it() {
    let factory = Factory::new("missing");

    let (status, body) = call(factory.router(), "/runs/run_nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json(&body)["detail"].as_str().unwrap().contains("run_nope"));
}

#[tokio::test]
async fn an_empty_history_serves_zeroes_rather_than_failing() {
    // The dashboard has to work on a factory that has never run, which is every factory on the
    // day it is written.
    let factory = Factory::new("fresh");
    assert!(factory.path().join("history").is_dir());

    let (status, body) = call(factory.router(), "/costs?window=last_30d").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["total"]["runs"], 0);
    assert_eq!(json(&body)["total"]["usd"], 0.0);
    assert_eq!(
        json(&body)["total"]["confidence"],
        "reported",
        "nothing is unaccounted for when nothing has run"
    );
}

#[tokio::test]
async fn a_failed_run_colours_its_agent_on_the_route_map() {
    let factory = Factory::new("colour");
    factory.write_run(&format!(
        r#"{{"run":"run_f","itinerary":"itn_1","agent":"developer","outcome":"failed","started_at":"{now}","finished_at":"{now}","source":"unreported"}}"#,
        now = jiff::Timestamp::now()
    ));

    let (_, body) = call(factory.router(), "/graph").await;
    let svg = json(&body)["mermaid"].as_str().expect("svg").to_owned();

    assert!(
        svg.contains(r#"class="node agent failed" id="a_developer""#),
        "{svg}"
    );
}

#[tokio::test]
async fn health_reports_a_ground_stop_that_was_engaged_by_hand() {
    // The kill switch is a file precisely so it survives a crash and can be set when nothing else
    // responds. Health used to report a hardcoded `false`, so an operator who had just pulled the
    // handle would be told nothing was stopped -- the one lie a safety rail must never tell.
    let factory = Factory::new("ground-stop");

    let (_, before) = call(factory.router(), "/health").await;
    assert_eq!(json(&before)["ground_stop"], false);

    fs::write(factory.path().join("ground-stop"), "halted by hand").expect("writes");

    let (_, after) = call(factory.router(), "/health").await;
    assert_eq!(json(&after)["ground_stop"], true);
}

async fn send(app: Router, payload: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/flights")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_owned()))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_trigger_is_queued_and_shows_up_as_waiting() {
    // The dashboard is otherwise read-only. This one control writes, and what it writes is a
    // *queued* flight: nothing dispatches it, because dispatching needs the supervisor.
    let factory = Factory::new("trigger");

    let (status, body) = send(
        factory.router(),
        r#"{"pipeline":"triage","body":"work item 42"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        json(&body)["to"],
        "analyst",
        "it is addressed to the entry agent"
    );

    let (_, queue) = call(factory.router(), "/flights").await;
    let pending = json(&queue)["pending"].as_array().expect("pending").clone();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["body"], "work item 42");
    assert!(
        json(&queue)["dispatched_by"].is_null(),
        "null is the honest answer while nothing will pick it up"
    );
}

#[tokio::test]
async fn a_queued_trigger_survives_a_restart() {
    // A queue held in memory would be a button that looks like it did something until the next
    // time the dashboard is started.
    let factory = Factory::new("trigger-durable");
    send(
        factory.router(),
        r#"{"pipeline":"triage","body":"work item 42"}"#,
    )
    .await;

    let (_, queue) = call(factory.router(), "/flights").await;

    assert_eq!(json(&queue)["pending"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_queued_trigger_keeps_the_flags_and_pipeline_it_was_asked_for() {
    // The flags are the whole point of the trigger dialog, and they are consumed at prompt
    // composition time — which, for queued work, has not happened yet and may be after a restart.
    // Accepting them and storing only the flight made the dialog report success while the run it
    // booked would have been composed as though the operator had set nothing.
    let factory = Factory::new("trigger-flags-kept");

    let (status, _) = send(
        factory.router(),
        r#"{"pipeline":"triage","body":"work item 42","flags":{"deep":true}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, queue) = call(factory.router(), "/flights").await;
    let queued = &json(&queue)["pending"][0];

    assert_eq!(
        queued["pipeline"], "triage",
        "a queued flight must remember how it was triggered: {queue}"
    );

    let flags = queued["flags"].as_array().expect("flags are reported");
    let deep = flags
        .iter()
        .find(|flag| flag["name"] == "deep")
        .unwrap_or_else(|| panic!("`deep` is missing from {queue}"));

    assert_eq!(deep["value"], true, "the operator set this: {queue}");
}

#[tokio::test]
async fn a_flag_the_pipeline_does_not_declare_is_refused() {
    // Silently dropping it would let a typo change nothing while appearing to work, which is the
    // same failure the per-entry-point flag check exists to prevent at load time.
    let factory = Factory::new("trigger-flag");

    let (status, body) = send(
        factory.router(),
        r#"{"pipeline":"triage","body":"go","flags":{"nonsense":true}}"#,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json(&body)["detail"].as_str().unwrap().contains("nonsense"),
        "{body}"
    );
}

#[tokio::test]
async fn triggering_an_unknown_pipeline_is_refused() {
    let factory = Factory::new("trigger-ghost");

    let (status, body) = send(factory.router(), r#"{"pipeline":"nope","body":"go"}"#).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json(&body)["detail"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn nothing_is_queued_while_a_ground_stop_is_engaged() {
    // A kill switch that stops running work but lets more be booked is not a kill switch.
    let factory = Factory::new("trigger-halted");
    fs::write(factory.path().join("ground-stop"), "halted").expect("writes");

    let (status, _) = send(factory.router(), r#"{"pipeline":"triage","body":"go"}"#).await;

    assert_eq!(status, StatusCode::CONFLICT);
    let (_, queue) = call(factory.router(), "/flights").await;
    assert_eq!(json(&queue)["pending"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn a_run_with_no_report_says_so_rather_than_failing_oddly() {
    let factory = Factory::new("report-missing");

    let (status, body) = call(factory.router(), "/runs/run_nope/report").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json(&body)["detail"].as_str().unwrap().contains("run_nope"));
}

#[tokio::test]
async fn the_reserve_is_metered_over_its_own_window_not_the_one_being_browsed() {
    // The Reserve caps spending per rolling window; the cost page shows whatever period the
    // operator picked. Computing both from one ledger compared thirty days of spend against a
    // twenty-four hour cap, so the rail reported itself exhausted on money it was never meant to
    // count — and a rail that reports a number which is not true is not a rail.
    let factory = Factory::new("reserve-window");
    factory.write_config(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[reserve]
fuel_usd = 10.0
window_hours = 24

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true
"#,
    );

    factory.write_run_ago("run_recent", 2, 3.0);
    factory.write_run_ago("run_old", 72, 50.0);

    let (_, body) = call(factory.router(), "/costs?window=last_30d").await;
    let report = json(&body);

    assert_eq!(
        report["total"]["usd"], 53.0,
        "the browsing window still shows everything in the last thirty days: {body}"
    );
    assert_eq!(
        report["reserve"]["spent_usd"], 3.0,
        "only the last 24 hours count against a 24-hour cap: {body}"
    );
    assert_eq!(
        report["reserve"]["remaining_usd"], 7.0,
        "$10 cap less the $3 spent inside the window: {body}"
    );
    assert_eq!(
        report["reserve"]["exhausted"], false,
        "$53 over thirty days must not exhaust a cap that only meters one day: {body}"
    );
}

#[tokio::test]
async fn costs_narrow_to_one_workflow_but_the_reserve_does_not() {
    // "What does the build cost" and "what may the factory still spend" are different questions.
    // Narrowing the page must not narrow the Reserve, or a workflow's own spend gets compared
    // against a cap that covers every workflow together.
    let factory = Factory::new("cost-scope");
    factory.write_config(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[reserve]
fuel_usd = 100.0
window_hours = 24

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true
"#,
    );

    factory.write_run_in("build", "run_b", 3.0);
    factory.write_run_in("sweep", "run_s", 7.0);

    let (_, all) = call(factory.router(), "/costs?window=last_7d").await;
    assert_eq!(json(&all)["total"]["usd"], 10.0, "{all}");

    let (_, build) = call(factory.router(), "/costs?window=last_7d&pipeline=build").await;
    let report = json(&build);

    assert_eq!(
        report["total"]["usd"], 3.0,
        "only the build's runs: {build}"
    );
    assert_eq!(
        report["by_agent"].as_array().map(Vec::len),
        Some(1),
        "the breakdown narrows with the total: {build}"
    );
    assert_eq!(
        report["reserve"]["spent_usd"], 10.0,
        "the Reserve caps the factory and must keep reporting the factory: {build}"
    );
}

#[tokio::test]
async fn help_is_attributed_to_the_workflow_that_raised_it() {
    // A help request records its itinerary, not its workflow. The runs in the window supply the
    // mapping, so nothing has to be stored twice and nothing has to be guessed.
    let factory = Factory::new("help-scope");
    factory.write_config(TWO_AGENTS);
    factory.write_run_in("build", "run_b", 1.0);
    factory.write_help("itn_build", "analyst", "the token expired");

    let (_, all) = call(factory.router(), "/help?open=true&window=last_7d").await;
    assert_eq!(json(&all)["requests"][0]["pipeline"], "build", "{all}");

    let (_, matching) = call(
        factory.router(),
        "/help?open=true&window=last_7d&pipeline=build",
    )
    .await;
    assert_eq!(
        json(&matching)["requests"].as_array().map(Vec::len),
        Some(1)
    );

    let (_, other) = call(
        factory.router(),
        "/help?open=true&window=last_7d&pipeline=sweep",
    )
    .await;
    assert_eq!(
        json(&other)["requests"].as_array().map(Vec::len),
        Some(0),
        "another workflow's help must not appear here: {other}"
    );
}
