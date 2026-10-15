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

/// A manifest announcing `version` with an archive for this platform named `archive`, whose
/// SHA-256 it lists as `sha256`.
pub fn manifest_with_archive(version: &str, archive: &str, sha256: &str) -> String {
    let executable = if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    };
    serde_json::json!({
        "announcement_tag": format!("v{version}"),
        "announcement_is_prerelease": false,
        "announcement_title": format!("{version} — 2026-10-17"),
        "announcement_changelog": "Notes.",
        "releases": [{ "app_name": "layover-cli", "app_version": version }],
        "artifacts": {
            archive: {
                "kind": "executable-zip",
                "target_triples": [layover_update::TARGET],
                "checksums": { "sha256": sha256 },
                "assets": [{ "kind": "executable", "path": executable, "name": executable }]
            }
        }
    })
    .to_string()
}

/// The archive's name for this platform, as dist names it.
pub fn archive_name() -> String {
    let ext = if cfg!(windows) { "zip" } else { "tar.xz" };
    format!("layover-cli-{}.{ext}", layover_update::TARGET)
}

/// An archive in this platform's format holding `entries`, each a path and its bytes.
pub fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    if cfg!(windows) {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            for (path, bytes) in entries {
                zip.start_file(*path, zip::write::SimpleFileOptions::default())
                    .expect("starts");
                std::io::Write::write_all(&mut zip, bytes).expect("writes");
            }
            zip.finish().expect("finishes");
        }
        out.into_inner()
    } else {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Regular);
            // Written raw, so a test can name a path the builder itself would refuse.
            let name = header.as_old_mut().name.as_mut();
            name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            tar.append(&header, *bytes).expect("appends");
        }
        let tar = tar.into_inner().expect("finishes");
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut std::io::Cursor::new(tar), &mut xz).expect("compresses");
        xz
    }
}

/// The SHA-256 of `bytes`, in hex.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl Fake {
    /// Publishes release `version` carrying `archive`, listed with `listed` as its SHA-256 and
    /// with `sidecar` in its `.sha256` file.
    pub fn publish_archive(&self, version: &str, archive: &[u8], listed: &str, sidecar: &str) {
        let tag = format!("v{version}");
        let name = archive_name();
        self.answer(
            &format!("/releases/download/{tag}/dist-manifest.json"),
            Answer::ok(manifest_with_archive(version, &name, listed)),
        );
        self.answer(
            &format!("/releases/download/{tag}/{name}"),
            Answer::ok(archive.to_vec()),
        );
        self.answer(
            &format!("/releases/download/{tag}/{name}.sha256"),
            Answer::ok(format!("{sidecar} *{name}\n")),
        );
    }
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
