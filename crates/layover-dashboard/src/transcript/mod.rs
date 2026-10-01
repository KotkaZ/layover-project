//! Turning what an agent CLI printed into what a person watching it would want to read.
//!
//! A run's transcript is everything its CLI wrote to stdout and stderr, as it wrote it. For Copilot
//! CLI and Claude Code that is one JSON event per line, and most of it is not for people: in a
//! real forty-minute Copilot review, 29 MB of the 44 MB were token-by-token deltas of text that
//! arrives again, whole, a moment later. Shown raw it is unreadable; shown whole it does not fit in
//! a browser.
//!
//! So a transcript is rendered here into **entries**, the way the CLI itself would show a session
//! in a terminal: what it was asked, what it thought, each tool it called and what came back, what
//! it said. Text still arriving is a **partial** — the block a CLI is typing into — which the
//! finished entry replaces. A partial and its entry carry the same id, so a viewer that missed
//! the deltas loses nothing.
//!
//! Output that is not one of the event dialects below — Codex, a shell stand-in, a CLI's own
//! stderr — is shown as it was written.
//!
//! Everything shown passes through the same redaction as a run's failure detail, and is cut to a
//! size a page can hold: the whole of a tool's output and the whole of the prompt are in the run's
//! Hangar for anyone who needs them.

mod claude;
mod copilot;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

/// How an entry is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// What the run was asked: its prompt, or a message delivered to it while it worked.
    Prompt,
    /// The model's reasoning.
    Think,
    /// What the agent said.
    Say,
    /// A tool it called, and with what.
    Tool,
    /// What a tool gave back.
    Done,
    /// A tool, or the run, that failed.
    Failed,
    /// What the CLI said about the session: the model, its servers, what it cost.
    Info,
    /// Output that was not an event: shown as written.
    Raw,
}

impl Kind {
    /// The name a viewer styles it by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Think => "think",
            Self::Say => "say",
            Self::Tool => "tool",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Info => "info",
            Self::Raw => "raw",
        }
    }
}

/// One finished thing a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// How to draw it.
    pub kind: Kind,
    /// What to show; may run to several lines.
    pub text: String,
    /// When the CLI said it happened, when it said.
    pub at: Option<String>,
    /// The partial this entry completes, which a viewer replaces with it.
    pub closes: Option<String>,
}

/// Text that is still arriving: the tail of it, as it stands. Empty when the block is finished or
/// was abandoned, which tells a viewer to take it away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    /// Which block, the same as the [`Entry::closes`] of the entry that finishes it.
    pub id: String,
    /// How to draw it.
    pub kind: Kind,
    /// The most recent part of it.
    pub text: String,
}

/// The longest line shown before it is cut.
const LINE_CHARS: usize = 400;
/// How much of a block still arriving is sent: the end of it, which is where the reader is.
const PARTIAL_CHARS: usize = 1_500;

/// Renders one transcript, a line at a time, keeping what it needs between lines.
#[derive(Debug, Default)]
pub struct Renderer {
    entries: Vec<Entry>,
    /// Blocks still arriving, by id.
    open: BTreeMap<String, (Kind, String)>,
    /// Blocks changed since a viewer was last told.
    touched: BTreeSet<String>,
    /// Blocks a viewer has been shown, so only those are taken away again.
    shown: BTreeSet<String>,
    /// What each tool call was, so its result can say.
    tools: HashMap<String, String>,
    /// The model last announced, so a change is said once.
    model: Option<String>,
    /// Plain output waiting to become one entry.
    raw: Vec<String>,
}

impl Renderer {
    /// A renderer at the start of a transcript.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads one line of the transcript, without its newline.
    pub fn line(&mut self, line: &str) {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            return;
        }

        let event = serde_json::from_str::<Value>(line).ok();
        let kind = event
            .as_ref()
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str);
        let (Some(event), Some(kind)) = (event.as_ref(), kind) else {
            self.raw.push(line.to_owned());
            return;
        };

        self.flush_raw();
        if claude::speaks(event, kind) {
            claude::event(self, event, kind);
        } else {
            copilot::event(self, event, kind);
        }
    }

    /// Everything finished since the last call, leaving what is still arriving to be told later.
    ///
    /// For a reader that has not caught up: a block that looks open at the end of one chunk of a
    /// long transcript is usually finished in the next, and showing it as typing would be wrong.
    pub fn entries(&mut self) -> Vec<Entry> {
        self.flush_raw();
        std::mem::take(&mut self.entries)
    }

    /// Everything finished since the last call, and every block still arriving that changed.
    pub fn take(&mut self) -> (Vec<Entry>, Vec<Partial>) {
        self.flush_raw();
        let mut partials = Vec::new();

        for id in std::mem::take(&mut self.touched) {
            match self.open.get(&id) {
                Some((kind, text)) => {
                    self.shown.insert(id.clone());
                    partials.push(Partial {
                        id,
                        kind: *kind,
                        text: redacted(tail(text, PARTIAL_CHARS)),
                    });
                }
                // Finished or abandoned. Only said for a block the viewer was shown: a catch-up
                // that read a whole block at once never put it on screen.
                None if self.shown.remove(&id) => partials.push(Partial {
                    id,
                    kind: Kind::Raw,
                    text: String::new(),
                }),
                None => {}
            }
        }

        (std::mem::take(&mut self.entries), partials)
    }

    /// The end of the transcript: a block that never finished is shown as far as it got.
    pub fn finish(&mut self) -> Vec<Entry> {
        self.flush_raw();
        for (id, (kind, text)) in std::mem::take(&mut self.open) {
            self.touched.insert(id.clone());
            let closes = self.shown.contains(&id).then(|| id.clone());
            self.emit(kind, &format!("{text} …"), None, closes);
        }
        self.take().0
    }

    /// Adds `delta` to the block `id`.
    pub(crate) fn grow(&mut self, id: String, kind: Kind, delta: &str) {
        self.open
            .entry(id.clone())
            .or_insert((kind, String::new()))
            .1
            .push_str(delta);
        self.touched.insert(id);
    }

    /// Replaces what the block `id` shows.
    pub(crate) fn set(&mut self, id: String, kind: Kind, text: &str) {
        self.open.insert(id.clone(), (kind, text.to_owned()));
        self.touched.insert(id);
    }

    /// Ends the block `id`, returning it when a viewer was shown it, so the entry that finishes it
    /// can take its place.
    pub(crate) fn close(&mut self, id: String) -> Option<String> {
        self.open.remove(&id);
        let shown = self.shown.contains(&id).then(|| id.clone());
        self.touched.insert(id);
        shown
    }

    /// Adds a finished entry, redacted and cut to size. Text that is only whitespace adds nothing.
    pub(crate) fn emit(
        &mut self,
        kind: Kind,
        text: &str,
        at: Option<&str>,
        closes: Option<String>,
    ) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let limit = match kind {
            Kind::Say | Kind::Raw => 400,
            Kind::Think => 200,
            Kind::Done | Kind::Failed => 8,
            Kind::Prompt => 3,
            Kind::Tool | Kind::Info => 4,
        };
        self.entries.push(Entry {
            kind,
            text: redacted(&clip(text, limit)),
            at: at.map(str::to_owned),
            closes,
        });
    }

    /// Remembers what a tool call was.
    pub(crate) fn called(&mut self, id: &str, name: &str) {
        self.tools.insert(id.to_owned(), name.to_owned());
    }

    /// What the tool call `id` was, if it was seen.
    pub(crate) fn tool_named(&self, id: &str) -> Option<&str> {
        self.tools.get(id).map(String::as_str)
    }

    /// Says which model is answering, once per change.
    pub(crate) fn model(&mut self, model: &str, at: Option<&str>) {
        if self.model.as_deref() != Some(model) {
            self.model = Some(model.to_owned());
            self.emit(Kind::Info, &format!("model {model}"), at, None);
        }
    }

    fn flush_raw(&mut self) {
        if !self.raw.is_empty() {
            let text = std::mem::take(&mut self.raw).join("\n");
            self.emit(Kind::Raw, &text, None, None);
        }
    }
}

/// A one-line account of what a tool was asked to do, from its arguments.
pub(crate) fn summary(arguments: &Value) -> String {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);

    if let Some(command) = text("command") {
        return format!("$ {}", first_line(command));
    }
    if let Some(path) = text("path").or_else(|| text("file_path")) {
        return match arguments.get("view_range").and_then(Value::as_array) {
            Some(range) if range.len() == 2 => format!("{path} ({}–{})", range[0], range[1]),
            _ => path.to_owned(),
        };
    }
    if let Some(pattern) = text("pattern") {
        return match arguments.get("paths").or_else(|| arguments.get("path")) {
            Some(Value::String(paths)) => format!("{pattern:?} in {paths}"),
            Some(Value::Array(paths)) if !paths.is_empty() => format!(
                "{pattern:?} in {}",
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => format!("{pattern:?}"),
        };
    }
    if let (Some(to), Some(body)) = (text("to"), text("body")) {
        return format!("→ {to}: {}", first_line(body));
    }
    for key in ["headline", "description", "query", "url", "text", "name"] {
        if let Some(value) = text(key) {
            return first_line(value);
        }
    }
    match arguments {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        other => cut(&other.to_string(), 160),
    }
}

/// The first line of `text` that says anything, marked when there was more.
pub(crate) fn first_line(text: &str) -> String {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let head = lines.next().unwrap_or_default().trim().to_owned();
    if lines.next().is_some() {
        format!("{head} …")
    } else {
        head
    }
}

/// The text of a tool's result, however the CLI shaped it: a string, or a list of text blocks.
pub(crate) fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str).or(block.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Keeps the first `lines` lines, each cut to [`LINE_CHARS`], and says how much was left out.
fn clip(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = all
        .iter()
        .take(lines)
        .map(|line| cut(line, LINE_CHARS))
        .collect();
    if all.len() > lines {
        out.push(format!("… {} more line(s)", all.len() - lines));
    }
    out.join("\n")
}

/// Cuts `text` to `chars` characters, saying that it did.
fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The last `chars` characters of `text`.
fn tail(text: &str, chars: usize) -> &str {
    let count = text.chars().count();
    if count <= chars {
        return text;
    }
    let start = text
        .char_indices()
        .nth(count - chars)
        .map_or(0, |(index, _)| index);
    &text[start..]
}

fn redacted(text: &str) -> String {
    layover_core::help::redact::secrets(text).into_owned()
}
