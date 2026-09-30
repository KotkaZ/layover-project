//! What the tests of runs in parallel share: throwaway factories, runners that take a known time
//! or cost a known amount, and history read back from disk.
//!
//! Each test binary compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_core::run::RunRecord;
use layover_tower::Factory;

pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-parallel-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A runner command that takes about `seconds` and exits cleanly.
pub fn sleeps(seconds: u32) -> String {
    if cfg!(windows) {
        // `ping -n N` waits about N - 1 seconds.
        format!(r#"["cmd", "/c", "ping -n {} 127.0.0.1 >nul"]"#, seconds + 1)
    } else {
        format!(r#"["sh", "-c", "sleep {seconds}"]"#)
    }
}

/// A runner command that prints a Copilot checkpoint worth $15.11 and exits.
pub fn costs(temp: &Temp) -> String {
    let stream = temp.0.join("stream.jsonl");
    std::fs::write(
        &stream,
        "{\"type\":\"session.usage_checkpoint\",\"data\":{\"totalNanoAiu\":1510581560000,\"totalPremiumRequests\":15}}\n",
    )
    .expect("writes");
    if cfg!(windows) {
        format!(r#"["cmd", "/c", "type", '{}']"#, stream.display())
    } else {
        format!(r#"["cat", '{}']"#, stream.display())
    }
}

pub fn factory(temp: &Temp, text: &str) -> Factory {
    let config: Config = toml::from_str(text).expect("the fixture parses");
    Factory::new(config, &temp.0).expect("opens")
}

pub fn to(chain: &ItineraryId, from: Origin, agent: &str) -> Queued {
    Queued::new(
        Flight::new(chain.clone(), from, AgentName::new(agent), "go", 6),
        None,
        BTreeMap::new(),
    )
}

pub fn human(agent: &str) -> Queued {
    to(&ItineraryId::generate(), Origin::Human, agent)
}

/// Every run in history.
pub fn history(root: &Path) -> Vec<RunRecord> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(".layover").join("history")) else {
        return found;
    };
    for entry in entries.flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        found.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<RunRecord>(line).ok()),
        );
    }
    found.sort_by_key(|record| record.started_at);
    found
}

/// The runs of one agent.
pub fn of<'a>(runs: &'a [RunRecord], agent: &'a str) -> impl Iterator<Item = &'a RunRecord> {
    runs.iter().filter(move |run| run.agent.as_str() == agent)
}

/// The most runs alive at once among `runs`.
pub fn overlap<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) -> usize {
    let runs: Vec<&RunRecord> = runs.into_iter().collect();
    runs.iter()
        .map(|at| {
            runs.iter()
                .filter(|other| {
                    other.started_at <= at.started_at
                        && other.finished_at.is_some_and(|end| end > at.started_at)
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// The live records a Tower left in the state directory.
pub fn live_records(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root.join(".layover").join("state").join("runs"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect()
        })
        .unwrap_or_default()
}

/// What each run of `agent` was told, oldest first.
pub fn payloads(root: &Path, agent: &str) -> Vec<String> {
    let mut found: Vec<(std::time::SystemTime, String)> =
        std::fs::read_dir(root.join(".layover").join("hangars").join(agent))
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path().join("prompt.md");
                        let text = std::fs::read_to_string(&path).ok()?;
                        let at = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
                        Some((at, text))
                    })
                    .collect()
            })
            .unwrap_or_default();
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, text)| text).collect()
}
