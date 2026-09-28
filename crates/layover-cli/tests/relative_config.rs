//! A factory started from a relative `--config` must hand its children paths that still resolve
//! from wherever those children run.
//!
//! These drive the real binary rather than calling `commands::run`, because the defect lives in
//! the gap between the Tower's working directory and the child's, and only a separate process has
//! a working directory of its own for the two to differ.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_store::Journal;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-relcfg-{name}-{}", std::process::id()));
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

/// A runner that succeeds only when the prompt file and the MCP configuration it was handed both
/// exist *from its own working directory*, which is the child's view and not the Tower's.
///
/// `{prompt}` is the first argument and the MCP flag and path are appended after it, so the paths
/// under test are the first and third.
fn checking_runner(dir: &Path) -> String {
    if cfg!(windows) {
        let script = dir.join("check.cmd");
        std::fs::write(
            &script,
            "@echo off\r\n\
             if not exist \"%~1\" (echo prompt not found: %1& exit /b 7)\r\n\
             if not exist \"%~3\" (echo MCP configuration not found: %3& exit /b 8)\r\n",
        )
        .expect("writes the runner");
        format!("command = ['{}', '{{prompt}}']", script.display())
    } else {
        r#"command = ["sh", "-c", "test -f \"$1\" || { echo prompt not found: $1; exit 7; }; test -f \"$3\" || { echo MCP configuration not found: $3; exit 8; }", "sh", "{prompt}"]"#
            .to_owned()
    }
}

/// Writes a one-agent factory into `factory` and queues one flight for it.
fn factory(factory: &Path, runner_dir: &Path, work_dir: Option<&Path>) {
    std::fs::create_dir_all(factory).expect("a factory directory");

    let work_dir = work_dir.map_or_else(String::new, |dir| {
        format!("[layover]\nwork_dir = '{}'\n", dir.display())
    });

    let text = format!(
        r#"{work_dir}
[defaults]
runner = "check"
timeout_sec = 60

[runners.check]
{}
mcp = {{ flag = "--mcp-config", format = "claude_json" }}

[agents.worker]
prompt = "work"
entry = true
"#,
        checking_runner(runner_dir)
    );
    std::fs::write(factory.join("layover.toml"), text).expect("writes the factory");

    let journal =
        Journal::open(factory.join(".layover").join("journal")).expect("opens the journal");
    journal
        .queue(Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("worker"),
                "go",
                4,
            ),
            None,
            BTreeMap::new(),
        ))
        .expect("queues a flight");
}

/// Runs `layover run --config <config>` from `cwd` and returns what it printed.
fn run(cwd: &Path, config: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_layover"))
        .arg("run")
        .arg("--config")
        .arg(config)
        .current_dir(cwd)
        .output()
        .expect("the binary runs");

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    text
}

/// The transcript of the one run, which is where a child's complaint ends up.
fn transcript(factory: &Path) -> String {
    let hangars = factory.join(".layover").join("hangars").join("worker");
    std::fs::read_dir(&hangars)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("transcript.log"))
        .find(|path| path.exists())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

#[test]
fn a_relative_config_with_an_outside_work_dir_hands_the_child_paths_it_can_open() {
    // The reported failure: every run died in a second because `@.layover\hangars\…\mcp.json` was
    // resolved against the agent's `work_dir`, which is somewhere else entirely.
    let temp = Temp::new("outside");
    let site = temp.0.join("factory");
    let outside = temp.0.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("an outside work_dir");
    factory(&site, &temp.0, Some(&outside));

    let printed = run(&site, Path::new("layover.toml"));

    assert!(
        printed.contains("worker succeeded"),
        "{printed}\ntranscript:\n{}",
        transcript(&site)
    );
}

#[test]
fn a_relative_config_with_the_default_work_dir_hands_the_child_paths_it_can_open() {
    // `work_dir = "workspace"` is the default, so this is what a plain `layover serve` does.
    let temp = Temp::new("default");
    let site = temp.0.join("factory");
    factory(&site, &temp.0, None);

    let printed = run(&site, Path::new("layover.toml"));

    assert!(
        printed.contains("worker succeeded"),
        "{printed}\ntranscript:\n{}",
        transcript(&site)
    );
}

#[test]
fn an_absolute_config_run_from_somewhere_else_works_too() {
    let temp = Temp::new("absolute");
    let site = temp.0.join("factory");
    let outside = temp.0.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("an outside work_dir");
    factory(&site, &temp.0, Some(&outside));

    let printed = run(&temp.0, &site.join("layover.toml"));

    assert!(
        printed.contains("worker succeeded"),
        "{printed}\ntranscript:\n{}",
        transcript(&site)
    );
}

#[test]
fn serve_names_absolute_paths_when_given_a_relative_config() {
    // An operator reading `History in .layover\history` cannot tell which factory that is, and a
    // relative path printed by a service started at logon means nothing at all.
    let temp = Temp::new("serve");
    let site = temp.0.join("factory");
    factory(&site, &temp.0, None);

    let mut child = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args([
            "serve",
            "--config",
            "layover.toml",
            "--addr",
            "127.0.0.1:0",
            "--no-auth",
            "--watch-only",
        ])
        .current_dir(&site)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let done = line.starts_with("History in ");
            let _ = tx.send(line);
            if done {
                break;
            }
        }
    });

    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(30)) {
        let done = line.starts_with("History in ");
        lines.push(line);
        if done {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    let named = |prefix: &str| {
        lines
            .iter()
            .find_map(|line| line.strip_prefix(prefix))
            .map_or_else(|| panic!("no `{prefix}` line in {lines:?}"), PathBuf::from)
    };

    for path in [named("Reading "), named("History in ")] {
        assert!(
            path.is_absolute(),
            "{} is relative: {lines:?}",
            path.display()
        );
        assert!(
            !path.display().to_string().starts_with(r"\\?\"),
            "a verbatim path is one many CLIs cannot open: {}",
            path.display()
        );
    }
}
