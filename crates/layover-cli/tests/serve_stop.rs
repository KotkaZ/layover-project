//! `layover stop` drains the Tower: what is running finishes, nothing new starts, and the process
//! exits cleanly. `--now` cuts it off instead: the run is killed, and left for the next Tower.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_store::Journal;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-serve-stop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("work")).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        // A run cut off may still hold the directory for a moment on Windows.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// A runner that takes `seconds`, then leaves `finished` in its working directory.
fn factory(seconds: u32) -> String {
    let sleeps = if cfg!(windows) {
        format!(
            r#"["cmd", "/c", "ping -n {} 127.0.0.1 >nul && echo done>finished"]"#,
            seconds + 1
        )
    } else {
        format!(r#"["sh", "-c", "sleep {seconds} && echo done > finished"]"#)
    };
    format!(
        r#"
[layover]
work_dir = "work"
http_addr = "127.0.0.1:0"

[defaults]
runner = "slow"
timeout_sec = 120

[runners.slow]
command = {sleeps}

[agents.slow]
prompt = "take a while"
entry = true
"#
    )
}

fn queue(root: &Path, agent: &str) {
    let journal = Journal::open(root.join(".layover").join("journal")).expect("opens");
    journal
        .queue(Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new(agent),
                "go",
                4,
            ),
            None,
            BTreeMap::new(),
        ))
        .expect("queues");
}

/// Starts `layover serve`, and a channel of every line it prints.
fn serve(root: &Path) -> (Child, Receiver<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(["serve", "--config", "layover.toml", "--no-auth"])
        // A test asks GitHub nothing.
        .env("LAYOVER_UPDATE", "off")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    (child, rx)
}

fn stop(root: &Path, now: bool) {
    let mut args = vec!["stop", "--config", "layover.toml"];
    if now {
        args.push("--now");
    }
    let status = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(&args)
        .current_dir(root)
        .status()
        .expect("stop runs");
    assert!(status.success());
}

/// Waits for the run to be alive: its live record is written before the process starts.
fn until_running(root: &Path) {
    let live = root.join(".layover").join("state").join("runs");
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        let alive = std::fs::read_dir(&live).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        });
        if alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no run started");
}

fn exits(child: &mut Child, within: Duration) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("waits") {
            return status;
        }
        if started.elapsed() > within {
            let _ = child.kill();
            panic!("serve did not exit within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Every line printed until one contains `text`.
fn until_said(rx: &Receiver<String>, text: &str) -> String {
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(30)) {
        let found = line.contains(text);
        lines.push(line);
        if found {
            return lines.join("\n");
        }
    }
    panic!("never said {text:?}: {}", lines.join("\n"));
}

fn said(rx: &Receiver<String>) -> String {
    rx.try_iter().collect::<Vec<_>>().join("\n")
}

fn history(root: &Path) -> String {
    std::fs::read_dir(root.join(".layover").join("history"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .collect::<String>()
        })
        .unwrap_or_default()
}

#[test]
fn layover_stop_lets_the_live_run_finish_and_starts_nothing_new() {
    let temp = Temp::new("drain");
    std::fs::write(temp.0.join("layover.toml"), factory(3)).expect("writes");
    queue(&temp.0, "slow");
    let (mut child, rx) = serve(&temp.0);

    until_running(&temp.0);
    stop(&temp.0, false);
    let mut output = until_said(&rx, "Stopping:");
    // Queued once the drain has begun: it must stay queued for the next Tower.
    queue(&temp.0, "slow");
    let status = exits(&mut child, Duration::from_secs(60));
    std::thread::sleep(Duration::from_millis(200));
    output.push('\n');
    output.push_str(&said(&rx));

    assert!(status.success(), "exited {status:?}: {output}");
    assert!(
        output.contains("Stopping: nothing new starts. Waiting for 1 run(s) to finish (slow)."),
        "{output}"
    );
    assert!(
        output.contains("Stopped: everything that was running has finished."),
        "{output}"
    );
    let ran = history(&temp.0);
    assert_eq!(ran.lines().count(), 1, "one run, and it finished: {ran}");
    assert!(ran.contains(r#""outcome":"succeeded""#), "{ran}");
    let journal = Journal::open(temp.0.join(".layover").join("journal")).expect("opens");
    assert_eq!(
        journal.pending().expect("reads").len(),
        1,
        "the second flight waits"
    );
    assert!(
        !temp.0.join(".layover").join("stop-request.json").exists(),
        "the request was honoured and removed"
    );
}

#[test]
fn layover_stop_now_cuts_the_run_off_for_the_next_tower_to_settle() {
    let temp = Temp::new("now");
    // Never restarted, so settling it does not run it again.
    let text = format!("{}recovery = \"never\"\n", factory(5));
    std::fs::write(temp.0.join("layover.toml"), text).expect("writes");
    queue(&temp.0, "slow");
    let (mut child, rx) = serve(&temp.0);

    until_running(&temp.0);
    stop(&temp.0, true);
    let status = exits(&mut child, Duration::from_secs(30));
    std::thread::sleep(Duration::from_millis(200));
    let output = said(&rx);

    // Stopped as asked, so a service manager that restarts a failed Tower leaves it stopped.
    assert!(status.success(), "exited {status:?}: {output}");
    assert!(output.contains("cutting off 1 run(s) (slow)"), "{output}");
    // Killed with the Tower rather than left to finish with nobody listening.
    std::thread::sleep(Duration::from_secs(7));
    assert!(
        !temp.0.join("work").join("finished").exists(),
        "the run outlived the Tower that cut it off"
    );
    assert!(
        history(&temp.0).is_empty(),
        "nothing finished, so nothing was recorded"
    );
    assert!(
        std::fs::read_dir(temp.0.join(".layover").join("state").join("runs"))
            .expect("live records")
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json")),
        "its live record is left for the next Tower to settle"
    );

    // What the next Tower does with it, without running anything new.
    let settled = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(["run", "--config", "layover.toml"])
        .current_dir(&temp.0)
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&settled.stdout);
    assert!(text.contains("was interrupted by a restart"), "{text}");
}
