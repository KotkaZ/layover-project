//! `layover serve` reads the machine it runs on, for the dashboard's System tab.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The status and body of `GET path` at `address`.
fn get(address: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).expect("connects");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .expect("asks");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("reads");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_default();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn a_dashboard_served_reads_the_machine() {
    let temp =
        Temp(std::env::temp_dir().join(format!("layover-serve-system-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&temp.0);
    std::fs::create_dir_all(&temp.0).expect("a temporary directory");
    std::fs::write(
        temp.0.join("layover.toml"),
        "[layover]\nwork_dir = \"work\"\n\n[agents.worker]\nprompt = \"work\"\n",
    )
    .expect("writes");

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
        .current_dir(&temp.0)
        .env("LAYOVER_UPDATE", "off")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(address) = line.strip_prefix("Layover dashboard on ") {
                let _ = tx.send(
                    address
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                        .to_owned(),
                );
                break;
            }
        }
    });
    let address = rx.recv_timeout(Duration::from_secs(30));
    // The first look at the machine is taken on its own thread, after the dashboard answers.
    let answer = address.as_deref().map(|address| {
        let mut answer = get(address, "/system");
        for _ in 0..100 {
            if answer.0 != 200 || answer.1.contains("\"holds\"") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            answer = get(address, "/system");
        }
        answer
    });
    let _ = child.kill();
    let _ = child.wait();

    let (status, body) = answer.expect("serve printed its address");
    assert_eq!(status, 200, "{body}");
    let report: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(report["window"], "last_hour");
    assert_eq!(report["disks"][0]["holds"][0], "factory", "{report}");
}
