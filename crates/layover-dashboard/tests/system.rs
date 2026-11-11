//! `GET /system`: the machine the dashboard runs on, read in the background, and Layover's share.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::Timestamp;
use layover_dashboard::{Dashboard, DashboardState, Guard, Monitor, router};
use layover_store::{History, Journal};
use serde_json::Value;
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
runner = "claude"
entry = true
"#;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-system-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self(root)
    }

    fn dashboard(&self) -> Dashboard {
        let layover = self.0.join(".layover");
        Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(layover.join("history")).expect("opens history"),
            journal: Arc::new(Journal::open(layover.join("journal")).expect("opens")),
            ground_stop: layover.join("ground-stop"),
        })
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn system(dashboard: &Dashboard, query: &str) -> (StatusCode, Value) {
    let response = router(dashboard.clone(), Guard::Open)
        .oneshot(
            Request::builder()
                .uri(format!("/system{query}"))
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
    (status, serde_json::from_slice(&bytes).expect("json"))
}

/// Asks until the monitor has read the machine `samples` times.
async fn read(dashboard: &Dashboard, samples: usize) -> Value {
    for _ in 0..100 {
        let (status, body) = system(dashboard, "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["samples"].as_array().map_or(0, Vec::len) >= samples {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the machine was not read {samples} times in ten seconds");
}

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

#[tokio::test]
async fn a_dashboard_with_nothing_reading_the_machine_says_so() {
    let factory = Factory::new("unread");
    let (status, body) = system(&factory.dashboard(), "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

#[tokio::test]
async fn the_machine_is_read_with_layovers_share_and_the_factorys_drive() {
    let factory = Factory::new("read");
    let monitor = Monitor::start_every(&factory.0.join("layover.toml"), Duration::from_millis(300));
    let dashboard = factory.dashboard().system(monitor);

    let report = read(&dashboard, 2).await;
    assert_eq!(report["window"], "last_hour");
    assert!(report["cpus"].as_i64() >= Some(1), "{report}");
    let memory = report["memory_total_bytes"].as_i64().expect("memory");
    assert!(memory > 0);

    // `work` is not made yet; the drive that will hold it is the factory's.
    let disks = report["disks"].as_array().expect("disks");
    assert_eq!(disks.len(), 1, "{report}");
    assert_eq!(
        disks[0]["holds"],
        serde_json::json!(["factory", "work_dir"])
    );
    let disk = disks[0]["total_bytes"].as_i64().expect("size");
    assert!(disk > 0);

    let since: Timestamp = report["since"]
        .as_str()
        .expect("since")
        .parse()
        .expect("valid");
    for sample in report["samples"].as_array().expect("samples") {
        let at: Timestamp = sample["at"].as_str().expect("at").parse().expect("valid");
        assert!(at >= since);
        assert!(
            (0.0..=100.0).contains(&number(&sample["cpu_percent"])),
            "{sample}"
        );
        assert!(
            (0.0..=100.0).contains(&number(&sample["layover_cpu_percent"])),
            "{sample}"
        );
        let used = sample["memory_used_bytes"].as_i64().expect("used");
        assert!(used > 0 && used <= memory, "{sample}");
        // This test is the process serving the dashboard, so its share counts at least itself.
        assert!(sample["layover_processes"].as_i64() >= Some(1), "{sample}");
        let held = sample["layover_memory_bytes"].as_i64().expect("held");
        assert!(held > 0 && held <= used, "{sample}");
        let on_disk = sample["disk_used_bytes"].as_array().expect("disks");
        assert_eq!(on_disk.len(), 1);
        assert!(on_disk[0].as_i64().expect("bytes") <= disk);
    }

    let (status, day) = system(&dashboard, "?window=last_24h").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(day["window"], "last_24h");
    assert_eq!(day["every_sec"], 60);
    assert!(
        !day["samples"].as_array().expect("samples").is_empty(),
        "the minute under way counts: {day}"
    );
}
