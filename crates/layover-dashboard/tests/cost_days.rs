//! Spend per day: what the Cost tab draws its chart from.

use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("layover-dash-days-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self(root)
    }

    /// Writes a run that finished `hours_ago`, costing `usd`.
    fn run(&self, run: &str, hours_ago: i64, usd: f64) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(hours_ago))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let path = self.0.join("history").join(format!("runs-{day}.jsonl"));
        let line = format!(
            r#"{{"run":"{run}","itinerary":"itn_{run}","agent":"analyst","outcome":"succeeded","started_at":"{at}","finished_at":"{at}","usd":{usd},"source":"reported"}}"#
        );
        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(&line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    async fn costs(&self, window: &str) -> serde_json::Value {
        let app = router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.0.join("history")).expect("opens history"),
                journal: std::sync::Arc::new(
                    Journal::open(self.0.join("journal")).expect("opens journal"),
                ),
                ground_stop: self.0.join("ground-stop"),
            }),
            Guard::Open,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/costs?window={window}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("responds");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("JSON")
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
"#;

/// The days of a report, as calendar dates.
fn days(report: &serde_json::Value) -> Vec<jiff::civil::Date> {
    report["by_day"]
        .as_array()
        .expect("by_day")
        .iter()
        .map(|day| day["day"].as_str().expect("day").parse().expect("a date"))
        .collect()
}

#[tokio::test]
async fn every_day_of_the_window_is_there_including_the_quiet_ones() {
    // A chart that left out the days nothing ran on would draw a factory busier than it was, and
    // one whose days skipped would put Monday's spend over Thursday's label.
    let factory = Factory::new("quiet");
    factory.run("run_today", 1, 2.0);
    factory.run("run_earlier", 72, 5.0);

    let report = factory.costs("last_7d").await;
    let days = days(&report);

    assert!(
        days.len() >= 7,
        "a week touches seven or eight days: {days:?}"
    );
    for pair in days.windows(2) {
        assert_eq!(pair[0].tomorrow().expect("a day"), pair[1], "{days:?}");
    }
    let today = jiff::Zoned::now().date();
    assert_eq!(
        days.last(),
        Some(&today),
        "the window ends today, where the Tower is"
    );
    let quiet = report["by_day"]
        .as_array()
        .expect("by_day")
        .iter()
        .filter(|day| day["summary"]["runs"] == 0)
        .count();
    assert!(quiet >= 4, "the days nothing ran on are kept: {report}");
    assert!(report["day_zone"].is_string() || report["day_zone"].is_null());
}

#[tokio::test]
async fn the_days_add_up_to_the_total() {
    let factory = Factory::new("sum");
    factory.run("run_a", 1, 2.0);
    factory.run("run_b", 30, 3.0);
    factory.run("run_c", 30, 0.5);

    let report = factory.costs("last_7d").await;
    let by_day = report["by_day"].as_array().expect("by_day");

    let runs: i64 = by_day
        .iter()
        .map(|day| day["summary"]["runs"].as_i64().expect("runs"))
        .sum();
    let usd: f64 = by_day
        .iter()
        .map(|day| day["summary"]["usd"].as_f64().expect("usd"))
        .sum();
    assert_eq!(runs, report["total"]["runs"].as_i64().expect("total runs"));
    assert!((usd - 5.5).abs() < 1e-9, "{report}");
}

#[tokio::test]
async fn all_time_begins_on_the_first_day_anything_was_spent() {
    // An open window has no start of its own; drawing from the beginning of retention would
    // stretch a factory installed yesterday across ninety empty days.
    let factory = Factory::new("open");
    factory.run("run_a", 1, 1.0);
    factory.run("run_b", 50, 1.0);

    let report = factory.costs("all_time").await;
    let days = days(&report);

    let first = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(50))
        .expect("in range")
        .to_zoned(jiff::tz::TimeZone::system())
        .date();
    assert_eq!(days.first(), Some(&first), "{days:?}");
    assert!(days.len() <= 4, "{days:?}");
}
