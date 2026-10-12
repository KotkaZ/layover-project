//! `GET /learnings` says when a lapsed learning will be deleted, by the factory's own
//! `[learnings] keep_lapsed_days`.

use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::Timestamp;
use layover_core::learning::{Impact, Learnings, PROVISIONAL_RUNS, Proposal};
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use serde_json::Value;
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[learnings]
keep_lapsed_days = 3

[agents.eagle]
prompt = "review"
"#;

struct Factory {
    root: PathBuf,
}

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-learnings-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self { root }
    }

    fn journal(&self) -> Journal {
        Journal::open(self.root.join(".layover").join("journal")).expect("opens")
    }

    async fn learnings(&self) -> Value {
        let layover = self.root.join(".layover");
        let dashboard = Dashboard::new(DashboardState {
            config_path: self.root.join("layover.toml"),
            history: History::open(layover.join("history")).expect("opens history"),
            journal: std::sync::Arc::new(self.journal()),
            ground_stop: layover.join("ground-stop"),
        });
        let response = router(dashboard, Guard::Open)
            .oneshot(
                Request::builder()
                    .uri("/learnings")
                    .body(Body::empty())
                    .expect("request builds"),
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
        serde_json::from_slice(&bytes).expect("json")
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn at(rfc3339: &str) -> Timestamp {
    rfc3339.parse().expect("valid timestamp")
}

#[tokio::test]
async fn a_lapsed_learning_says_when_it_will_be_deleted_and_a_live_one_says_nothing() {
    let factory = Factory::new("deleted-after");
    let mut learnings = Learnings::new();
    for text in ["skip draft pull requests", "pin the toolchain first"] {
        learnings.propose(&Proposal::new(
            "eagle".into(),
            text,
            Impact::Medium,
            at("2026-10-01T09:00:00Z"),
        ));
        if text.starts_with("skip") {
            for _ in 0..PROVISIONAL_RUNS {
                learnings.charge_run(&"eagle".into(), at("2026-10-02T09:00:00Z"));
            }
        }
    }
    factory.journal().save_learnings(&learnings).expect("saves");

    let listed = factory.learnings().await;
    let by_text = |text: &str| {
        listed["learnings"]
            .as_array()
            .and_then(|all| all.iter().find(|l| l["text"] == text).cloned())
            .unwrap_or_else(|| panic!("{text} is listed: {listed}"))
    };

    let lapsed = by_text("skip draft pull requests");
    assert_eq!(lapsed["state"], "lapsed");
    assert_eq!(lapsed["lapsed_at"], "2026-10-02T09:00:00Z");
    assert_eq!(lapsed["deleted_after"], "2026-10-05T09:00:00Z");

    let live = by_text("pin the toolchain first");
    assert_eq!(live["state"], "provisional");
    assert!(live["lapsed_at"].is_null(), "{live}");
    assert!(live["deleted_after"].is_null(), "{live}");
}
