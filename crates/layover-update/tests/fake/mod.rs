//! A release server on loopback, laid out as GitHub's releases are, that remembers every request.
//!
//! Each test binary compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

/// One answer the server gives.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn redirect(to: &str) -> Self {
        Self {
            status: 302,
            headers: vec![("Location".to_owned(), to.to_owned())],
            body: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// A request the server saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Default)]
struct Shared {
    answers: BTreeMap<String, Answer>,
    seen: Vec<Seen>,
}

/// The server. Its thread lives as long as the test binary.
#[derive(Clone)]
pub struct Fake {
    pub base: String,
    shared: Arc<Mutex<Shared>>,
}

impl Fake {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        let base = format!("http://{}", listener.local_addr().expect("bound"));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let serving = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&serving);
                std::thread::spawn(move || answer(stream, &shared));
            }
        });
        Self { base, shared }
    }

    /// The releases base a mirror is configured with.
    pub fn releases(&self) -> String {
        format!("{}/releases", self.base)
    }

    pub fn answer(&self, path: &str, answer: Answer) {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .answers
            .insert(path.to_owned(), answer);
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .seen
            .clone()
    }

    /// Publishes `manifest` as release `tag`, with an entity tag and an upload time, and makes it
    /// the newest stable release when `latest`.
    pub fn publish(&self, tag: &str, manifest: &str, uploaded: &str, latest: bool) {
        let path = format!("/releases/download/{tag}/dist-manifest.json");
        self.answer(
            &path,
            Answer::ok(manifest)
                .header("ETag", &format!("\"{tag}\""))
                .header("Last-Modified", uploaded),
        );
        if latest {
            self.answer(
                "/releases/latest/download/dist-manifest.json",
                Answer::redirect(&format!("{}{path}", self.base)),
            );
        }
    }
}

fn answer(stream: TcpStream, shared: &Mutex<Shared>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let reply = {
        let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let found = shared.answers.get(&path).cloned();
        let etag_matches = found.as_ref().is_some_and(|answer| {
            answer.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("etag") && headers.get("if-none-match") == Some(value)
            })
        });
        shared.seen.push(Seen { path, headers });
        match found {
            Some(_) if etag_matches => Answer::status(304),
            Some(answer) => answer,
            None => Answer::status(404),
        }
    };

    let mut stream = reader.into_inner();
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
}

/// A manifest announcing `version`, with `notes`.
pub fn manifest(version: &str, prerelease: bool, notes: &str) -> String {
    serde_json::json!({
        "announcement_tag": format!("v{version}"),
        "announcement_is_prerelease": prerelease,
        "announcement_title": format!("{version} — 2026-10-17"),
        "announcement_changelog": notes,
        "releases": [{ "app_name": "layover-cli", "app_version": version }],
        "artifacts": {}
    })
    .to_string()
}

/// A directory under the temp directory, removed when dropped.
pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-update-{tag}-{}", std::process::id()));
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
