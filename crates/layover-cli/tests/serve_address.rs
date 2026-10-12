//! `layover serve` binds where the factory says, unless `--addr` says otherwise.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-serve-addr-{tag}-{}", std::process::id()));
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

/// Starts `layover serve --watch-only` in `dir` with `extra` arguments, and returns the address
/// it says it is serving on.
fn served_on(dir: &Path, extra: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args([
            "serve",
            "--config",
            "layover.toml",
            "--no-auth",
            "--watch-only",
        ])
        .args(extra)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("serve starts");

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(address) = line.strip_prefix("Layover dashboard on ") {
                let _ = tx.send(address.to_owned());
                break;
            }
        }
    });

    let address = rx.recv_timeout(Duration::from_secs(30));
    let _ = child.kill();
    let output = child.wait_with_output().expect("waits");
    address.unwrap_or_else(|_| {
        panic!(
            "serve printed no address: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn port(address: &str) -> u16 {
    address
        .trim_end_matches('/')
        .rsplit(':')
        .next()
        .and_then(|port| port.split('/').next())
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("no port in {address}"))
}

const FACTORY: &str = r#"
[layover]
work_dir = "work"
http_addr = "127.0.0.1:0"

[agents.worker]
prompt = "work"
"#;

#[test]
fn serve_binds_the_address_the_factory_names() {
    // Port 0 asks the system for a free one, so the default 7878 is plainly not what bound.
    let temp = Temp::new("config");
    std::fs::write(temp.0.join("layover.toml"), FACTORY).expect("writes");

    let address = served_on(&temp.0, &[]);

    assert!(address.starts_with("http://127.0.0.1:"), "{address}");
    assert_ne!(port(&address), 7878, "{address}");
}

#[test]
fn addr_on_the_command_line_wins_over_the_factory() {
    let temp = Temp::new("flag");
    std::fs::write(
        temp.0.join("layover.toml"),
        FACTORY.replace("127.0.0.1:0", "127.0.0.1:1"),
    )
    .expect("writes");

    // If the factory's address won, serve would print port 1 — or, where port 1 is privileged,
    // fail to bind and print nothing.
    let address = served_on(&temp.0, &["--addr", "127.0.0.1:0"]);

    assert_ne!(port(&address), 1, "{address}");
}
