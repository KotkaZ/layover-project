//! `GET /pipelines` offers each workflow its own flags and the shared ones its prompts test.

use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use serde_json::Value;
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"
prompt_dir = "prompts"

[flags]
publish_pr = { default = false, description = "Publish the pull request" }

[agents.publisher]
prompt_file = "publisher.md"

[agents.monitor]
prompt_file = "monitor.md"

[pipelines.development]
entry = "publisher"

[pipelines.development.flags]
dry_run = { default = true }

[pipelines.watch]
entry = "monitor"
"#;

struct Factory(PathBuf);

impl Factory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("layover-dash-flags-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("dirs");
        fs::create_dir_all(root.join("prompts")).expect("dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes");
        fs::write(
            root.join("prompts").join("publisher.md"),
            "Publish.\n@include(publish_pr) open.md\n",
        )
        .expect("writes");
        fs::write(root.join("prompts").join("open.md"), "Open it.\n").expect("writes");
        fs::write(root.join("prompts").join("monitor.md"), "Watch.\n").expect("writes");
        Self(root)
    }

    async fn pipelines(&self) -> Value {
        let layover = self.0.join(".layover");
        let dashboard = Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(layover.join("history")).expect("opens"),
            journal: std::sync::Arc::new(Journal::open(layover.join("journal")).expect("opens")),
            ground_stop: layover.join("ground-stop"),
        });
        let response = router(dashboard, Guard::Open)
            .oneshot(
                Request::builder()
                    .uri("/pipelines")
                    .body(Body::empty())
                    .expect("builds"),
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
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn each_workflow_is_offered_only_the_shared_flags_its_prompts_test() {
    let factory = Factory::new();

    let listed = factory.pipelines().await;
    let flags_of = |name: &str| -> Vec<String> {
        let pipeline = listed["pipelines"]
            .as_array()
            .and_then(|all| all.iter().find(|p| p["name"] == name))
            .unwrap_or_else(|| panic!("{name} is listed: {listed}"));
        pipeline["flags"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|flag| flag["name"].as_str().expect("named").to_owned())
            .collect()
    };

    assert_eq!(flags_of("development"), ["dry_run", "publish_pr"]);
    assert!(flags_of("watch").is_empty(), "{listed}");
}
