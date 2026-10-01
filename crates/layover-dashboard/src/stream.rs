//! Following a run's transcript, as server-sent events.
//!
//! The Tower streams every run's output to a file in its Hangar as it is written, which is what
//! makes this possible from any process that can read the factory's directory: the dashboard does
//! not need the Tower to tell it anything. It reads the file from where the viewer left off,
//! renders what is new, and sends it. While the run is alive it looks again twice a second; when
//! the run's live record goes, it reads the rest, says how the run ended, and closes.
//!
//! # Events
//!
//! - `entry` — something the run finished doing: `{kind, text, at, closes}`. See
//!   [`crate::transcript`].
//! - `partial` — text still arriving: `{id, kind, text}`. Empty text takes it away.
//! - `end` — the run is over: `{status, detail}`, from history. Sent once, last.
//!
//! Every event carries an `id`: how far into the transcript it was read. A viewer that loses the
//! connection asks again with `?after=` that id, and is sent only what it has not seen.

use std::convert::Infallible;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use layover_core::RunId;
use layover_core::cost::Window;
use layover_store::live::Ledger;
use layover_store::{History, RunFilter};
use serde_json::json;

use crate::transcript::{Entry, Partial, Renderer};

/// How often a live run's transcript is looked at again.
const POLL: Duration = Duration::from_millis(500);
/// How long a quiet connection goes without a word, so a viewer that has gone is noticed.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// How much is read in one go. A forty-minute run's transcript is tens of megabytes, and reading
/// it whole would hold up every other request the dashboard is serving.
const CHUNK: u64 = 1 << 20;
/// The longest single line read before it is shown as it is rather than waited for.
const LONGEST_LINE: u64 = 32 << 20;

/// Where a run's output is, and how to tell when it is over.
pub(crate) struct Source {
    /// The transcript.
    pub transcript: PathBuf,
    /// The run.
    pub run: RunId,
    /// The live records, while the run is alive; `None` for a run already over.
    pub live: Option<Ledger>,
    /// History, for saying how it ended.
    pub history: History,
}

/// A response body that follows `source` from byte `after` until the run is over.
pub(crate) fn body(source: Source, after: u64) -> Body {
    let follow = Follow {
        offset: after,
        renderer: Renderer::new(),
        source,
        done: false,
        quiet_since: Instant::now(),
        started: after == 0,
    };
    Body::from_stream(futures_util::stream::unfold(follow, step))
}

struct Follow {
    source: Source,
    offset: u64,
    renderer: Renderer,
    done: bool,
    quiet_since: Instant,
    /// Whether this read began at the start, where a resumed one begins mid-line.
    started: bool,
}

async fn step(mut follow: Follow) -> Option<(Result<Bytes, Infallible>, Follow)> {
    if follow.done {
        return None;
    }

    loop {
        // Asked before reading, so whatever the run wrote before its record went is read on the
        // pass that notices it is over.
        let alive = follow
            .source
            .live
            .as_ref()
            .is_some_and(|ledger| ledger.find(&follow.source.run).is_some());

        let (lines, more) = follow.read(alive);
        for line in &lines {
            follow.renderer.line(line);
        }
        // Text still arriving is only told once the reader has caught up with a run that is still
        // going: before that, an open block is just one that the next chunk finishes.
        let (mut entries, partials) = if alive && !more {
            follow.renderer.take()
        } else {
            (follow.renderer.entries(), Vec::new())
        };

        if !alive && !more {
            entries.extend(follow.renderer.finish());
            let mut out = events(&entries, &[], follow.offset);
            out.push_str(&end(&follow.source, follow.offset, follow.missing()));
            follow.done = true;
            return Some((Ok(Bytes::from(out)), follow));
        }

        let out = events(&entries, &partials, follow.offset);
        if !out.is_empty() {
            follow.quiet_since = Instant::now();
            return Some((Ok(Bytes::from(out)), follow));
        }
        if more {
            continue;
        }
        if follow.quiet_since.elapsed() >= KEEP_ALIVE {
            follow.quiet_since = Instant::now();
            return Some((Ok(Bytes::from_static(b": still running\n\n")), follow));
        }
        tokio::time::sleep(POLL).await;
    }
}

impl Follow {
    /// The complete lines written since the last read, and whether there is more to read now.
    ///
    /// While the run is alive only whole lines are taken: the CLI may be halfway through writing
    /// the last one. Once it is over, whatever is left is the end of the transcript.
    fn read(&mut self, alive: bool) -> (Vec<String>, bool) {
        let Ok(mut file) = std::fs::File::open(&self.source.transcript) else {
            return (Vec::new(), false);
        };
        let length = file.metadata().map_or(0, |meta| meta.len());
        if self.offset > length {
            self.offset = length;
        }

        let mut limit = CHUNK;
        loop {
            let mut buffer = Vec::new();
            let read = file
                .seek(SeekFrom::Start(self.offset))
                .and_then(|_| file.by_ref().take(limit).read_to_end(&mut buffer));
            if read.is_err() || buffer.is_empty() {
                return (Vec::new(), false);
            }

            let reached_end = self.offset + buffer.len() as u64 >= length;
            let whole = match buffer.iter().rposition(|byte| *byte == b'\n') {
                Some(last) => last + 1,
                None if reached_end && !alive => buffer.len(),
                None if reached_end => return (Vec::new(), false),
                // One line longer than the chunk: read further, up to a limit past which it is
                // shown as far as it goes rather than waited for forever.
                None if limit < LONGEST_LINE => {
                    limit *= 2;
                    continue;
                }
                None => buffer.len(),
            };
            let take = if reached_end && !alive {
                buffer.len()
            } else {
                whole
            };

            let mut text = String::from_utf8_lossy(&buffer[..take]).into_owned();
            // A viewer resuming mid-line, given an offset that is not one this sent, starts at the
            // next whole line rather than rendering half of one.
            if !self.started {
                self.started = true;
                if self.offset > 0 && !preceded_by_newline(&mut file, self.offset) {
                    text = text
                        .split_once('\n')
                        .map(|(_, rest)| rest.to_owned())
                        .unwrap_or_default();
                }
            }

            self.offset += take as u64;
            // More only when this stopped at the chunk, not at the end of the file: a live run's
            // unfinished last line is not more to read yet.
            return (text.lines().map(str::to_owned).collect(), !reached_end);
        }
    }

    /// Whether there was never a transcript to read.
    fn missing(&self) -> bool {
        !self.source.transcript.exists()
    }
}

fn preceded_by_newline(file: &mut std::fs::File, offset: u64) -> bool {
    let mut byte = [0_u8];
    file.seek(SeekFrom::Start(offset - 1))
        .and_then(|_| file.read_exact(&mut byte))
        .is_ok_and(|()| byte[0] == b'\n')
}

/// Entries and partials as server-sent events, each marked with how far the transcript was read.
fn events(entries: &[Entry], partials: &[Partial], offset: u64) -> String {
    let mut out = String::new();
    for entry in entries {
        let data = json!({
            "kind": entry.kind.name(),
            "text": entry.text,
            "at": entry.at,
            "closes": entry.closes,
        });
        let _ = write!(out, "id: {offset}\nevent: entry\ndata: {data}\n\n");
    }
    for partial in partials {
        let data = json!({ "id": partial.id, "kind": partial.kind.name(), "text": partial.text });
        let _ = write!(out, "id: {offset}\nevent: partial\ndata: {data}\n\n");
    }
    out
}

/// The last event: how the run ended, as history recorded it.
fn end(source: &Source, offset: u64, missing: bool) -> String {
    // A run that has just ended was filed in today's segment; look there first, and only then
    // through everything kept.
    let find = |window| {
        let span = source.history.resolve(window);
        source
            .history
            .runs(&span, &RunFilter::default())
            .ok()?
            .into_iter()
            .find(|record| record.run == source.run)
    };
    let record = find(Window::Last24Hours).or_else(|| find(Window::AllTime));

    let mut data = json!({
        "status": record.as_ref().map(|record| crate::view::run(record).status),
        "detail": record.as_ref().and_then(|record| record.detail.clone()),
    });
    if missing {
        data["missing"] = json!(true);
    }
    format!("id: {offset}\nevent: end\ndata: {data}\n\n")
}
