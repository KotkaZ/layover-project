//! The queue says what will start it, and how many runs are alive against the factory's limit.

use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str, config: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("layover-dash-slots-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), config).expect("writes config");
        Self(root)
    }

    fn dashboard(&self) -> Dashboard {
        let layover = self.0.join(".layover");
        Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(layover.join("history")).expect("opens history"),
            journal: std::sync::Arc::new(Journal::open(layover.join("journal")).expect("opens")),
            ground_stop: layover.join("ground-stop"),
        })
    }

    /// Writes the live records a Tower keeps while its runs are alive, and what else it keeps
    /// beside them.
    fn alive(&self, runs: usize) {
        let dir = self.0.join(".layover").join("state").join("runs");
        fs::create_dir_all(dir.join("owners")).expect("dirs");
        fs::write(dir.join("owners").join("tower-1.lock"), "").expect("writes");
        for run in 0..runs {
            fs::write(dir.join(format!("run_{run}.json")), "{}").expect("writes");
        }
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
max_concurrent_runs = 3

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true
"#;

async fn flights(app: Router) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .uri("/flights")
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

#[tokio::test]
async fn a_dashboard_whose_process_runs_the_queue_names_it_and_counts_the_runs_alive() {
    // The live Tower that said "Nothing will dispatch them until a supervisor exists".
    let factory = Factory::new("serving", FACTORY);
    factory.alive(2);

    let app = router(
        factory
            .dashboard()
            .dispatched_by("the Tower in `layover serve` (process 7)"),
        Guard::Open,
    );
    let listed = flights(app).await;

    assert_eq!(
        listed["dispatched_by"],
        "the Tower in `layover serve` (process 7)"
    );
    assert_eq!(listed["alive_runs"], 2, "{listed}");
    assert_eq!(listed["max_concurrent_runs"], 3);
}

#[tokio::test]
async fn a_watching_dashboard_says_nothing_here_starts_the_queue() {
    let factory = Factory::new(
        "watching",
        &FACTORY.replace("max_concurrent_runs = 3\n", ""),
    );

    let listed = flights(router(factory.dashboard(), Guard::Open)).await;

    assert!(listed["dispatched_by"].is_null(), "{listed}");
    assert_eq!(listed["alive_runs"], 0);
    assert_eq!(listed["max_concurrent_runs"], 4, "the documented default");
}
