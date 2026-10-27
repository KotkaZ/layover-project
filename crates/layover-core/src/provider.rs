//! Telling a run the model service ended from a run that failed its own work.
//!
//! A Copilot run that has finished a 35-minute verification matrix can still die on its next model
//! call: the service answers `400 internal server error`, Copilot CLI does not retry a 400, and the
//! process exits 1. Recorded as `failed` like any other, it looks exactly like a run whose tests
//! failed — and the two want opposite things. A run that failed its own work must not be started
//! again, or a factory loops on a broken test; a run the service dropped is the work interrupted,
//! and is restarted like one. See [`crate::handover`].
//!
//! # Read from events, never from text
//!
//! The classification reads the structured events a CLI prints, one JSON object per line, and only
//! their top-level fields. An agent quoting an error — a tool result holding a log that says
//! `session.error`, a message explaining a 400 — is a string *inside* another event, and never
//! matches. A regular expression over output would match both, and would restart a run because it
//! read about a failure.
//!
//! # One line of text, at start-up
//!
//! Copilot CLI checks the model it was given before it makes a call, and a model the service is
//! not offering at that moment — an outage can leave one unlisted for minutes — ends the run with a
//! line of plain text instead of an event: `Error: Model "gpt-6.1-sol" from --model flag is not
//! available.` The run never started its work, so it was interrupted, not failed. That line counts
//! only as a whole line of the transcript, in a wording listed in [`MODEL_UNAVAILABLE`]: an agent
//! quoting it does so inside an event, whose line starts with `{` and is never compared. A model
//! name the CLI does not have at all fails the same way every time; the restart's attempts and
//! back-off bound that, and the CLI's own words stay in the run's record.
//!
//! # The failure has to be the last thing that happened
//!
//! Copilot CLI recovers from many failed calls by itself, and a sub-agent's failure is reported
//! with the same event as the session's own. So a failure counts only when nothing shows the run
//! carried on after it: no tool started, no model call succeeded. A run that hit a 400, waited for a
//! background task, was woken by it and finished its work, then exited 1 because of the earlier
//! error, did its work, and is not restarted.

use std::fmt;

use serde::Deserialize;
use serde_json::{Map, Value};

/// How a runner's output is read for a model-service failure, as `layover.toml` declares it.
///
/// Either a reader Layover has — `read = "copilot"`, or `read = "none"` never to classify — or the
/// events that say one, for a CLI Layover has no reader for:
///
/// ```toml
/// [runners.mine.provider_errors]
/// failed     = [{ type = "error", error = { kind = "provider" } }]
/// carried_on = [{ type = "tool_call" }, { type = "message" }]
/// request_id = "error.request_id"
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderErrors {
    /// A reader Layover has. Given instead of [`Self::failed`], not beside it.
    #[serde(default)]
    pub read: Option<Reader>,
    /// Events that say a model call failed: each one's fields, which an event must all have with
    /// these values. A table matches a nested object the same way.
    #[serde(default)]
    pub failed: Vec<Shape>,
    /// Events that say the run carried on after a failure — a tool started, a call succeeded —
    /// so the failure before them did not end it.
    #[serde(default)]
    pub carried_on: Vec<Shape>,
    /// Where in a [`Self::failed`] event the provider's request id is, as a dotted path.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// A reader Layover has for a CLI's events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reader {
    /// Copilot CLI's `--output-format json`.
    Copilot,
    /// Never classify: every non-zero exit is the run's own failure.
    None,
}

/// The fields an event must have, with these values, to match.
pub type Shape = Map<String, Value>;

/// How one runner's output is read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading<'a> {
    /// As Copilot CLI's JSON events.
    Copilot,
    /// With the events a runner declared.
    Declared(&'a ProviderErrors),
    /// Not at all: Layover has no reader for this CLI, or the runner said not to.
    Nothing,
}

/// A run the model service ended, and what the CLI said about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderFailure {
    /// What the CLI called the failure, such as `http_400` or `transport_error`.
    pub result: Option<String>,
    /// The provider's identifier for the request that failed, which its support asks for.
    pub request_id: Option<String>,
    /// What the CLI said, such as `Execution failed: 400 internal server error (Request ID: …)`.
    pub message: Option<String>,
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the model service failed")?;
        match (&self.result, &self.message) {
            (Some(result), Some(message)) => write!(f, " ({result}): {message}"),
            (Some(result), None) => write!(f, " ({result})"),
            (None, Some(message)) => write!(f, ": {message}"),
            (None, None) => Ok(()),
        }?;
        // The id is in the message when Copilot CLI wrote one; said again only when it is not.
        match &self.request_id {
            Some(id)
                if !self
                    .message
                    .as_deref()
                    .is_some_and(|m| m.contains(id.as_str())) =>
            {
                write!(f, " (request {id})")
            }
            _ => Ok(()),
        }
    }
}

impl Reading<'_> {
    /// The model-service failure that ended a run, if its `transcript` shows one.
    ///
    /// Call it only for a run that exited non-zero: a run that succeeded was not ended by
    /// anything, whatever it printed on the way.
    #[must_use]
    pub fn classify(&self, transcript: &str) -> Option<ProviderFailure> {
        match self {
            Self::Copilot => copilot(transcript),
            Self::Declared(declared) => declared_events(declared, transcript),
            Self::Nothing => None,
        }
    }
}

/// The events Copilot CLI's classification looks at. A line naming none of them cannot change
/// the answer, so it is not parsed: a forty-minute run prints tens of megabytes.
const COPILOT_EVENTS: [&str; 4] = [
    "tool.execution_start",
    "model.call_final_result",
    "session.error",
    "model.call_failure",
];

/// The words Copilot CLI ends a run with at start-up when the model it was given is not available:
/// each known wording, as what comes before the model's name and what comes after it, on a line of
/// its own. The first was printed on 26 October 2026, when a short outage left the model a run was
/// started with unlisted; a later wording is added here, with a test of its own.
pub const MODEL_UNAVAILABLE: [(&str, &str); 1] =
    [("Error: Model \"", "\" from --model flag is not available.")];

/// What the CLI said, without its `Error: `, when `line` is, whole, Copilot CLI saying at start-up
/// that the model it was given is not available.
fn model_unavailable(line: &str) -> Option<&str> {
    let line = line.trim();
    MODEL_UNAVAILABLE.iter().find_map(|(before, after)| {
        let model = line.strip_prefix(before)?.strip_suffix(after)?;
        (!model.is_empty() && !model.contains('"'))
            .then(|| line.strip_prefix("Error: ").unwrap_or(line))
    })
}

/// Reads Copilot CLI's events from the end: the failure that ended the run is after the last
/// sign it carried on — a tool starting, or a model call that succeeded.
///
/// A failure is a `model.call_final_result` whose `result` is not `success`, a `session.error`
/// whose `errorType` is `query`, both, or the line Copilot CLI prints at start-up for a model that
/// is not available. Read backwards, the first of each wins, because the first met is the latest.
fn copilot(transcript: &str) -> Option<ProviderFailure> {
    let mut found: Option<ProviderFailure> = None;

    for line in transcript.lines().rev() {
        if let Some(said) = model_unavailable(line) {
            let failure = found.get_or_insert_with(ProviderFailure::default);
            failure.message.get_or_insert_with(|| said.to_owned());
            continue;
        }
        if !COPILOT_EVENTS.iter().any(|kind| line.contains(kind)) {
            continue;
        }
        let Some(event) = event(line) else {
            continue;
        };
        let data = event.get("data").unwrap_or(&Value::Null);
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);

        match event.get("type").and_then(Value::as_str) {
            Some("tool.execution_start") => return found,
            Some("model.call_final_result") => match text("result") {
                Some(result) if result == "success" => return found,
                Some(result) => {
                    let failure = found.get_or_insert_with(ProviderFailure::default);
                    failure.result.get_or_insert(result);
                }
                None => {}
            },
            Some("session.error") if text("errorType").as_deref() == Some("query") => {
                let failure = found.get_or_insert_with(ProviderFailure::default);
                if let Some(message) = text("message") {
                    failure.message.get_or_insert(message);
                }
                if let Some(id) = text("providerCallId").or_else(|| text("serviceRequestId")) {
                    failure.request_id.get_or_insert(id);
                }
            }
            // The failing call's own record, before its final result: the request id, when the
            // session error that would carry it never came.
            Some("model.call_failure") => {
                if let (Some(failure), Some(id)) = (found.as_mut(), text("providerCallId")) {
                    failure.request_id.get_or_insert(id);
                }
            }
            _ => {}
        }
    }

    found
}

/// Reads a runner's declared events from the end, the way [`copilot`] reads Copilot CLI's.
fn declared_events(declared: &ProviderErrors, transcript: &str) -> Option<ProviderFailure> {
    if declared.failed.is_empty() {
        return None;
    }
    let mut found: Option<ProviderFailure> = None;

    for line in transcript.lines().rev() {
        let Some(event) = event(line) else {
            continue;
        };
        if declared.failed.iter().any(|shape| matches(&event, shape)) {
            let failure = found.get_or_insert_with(ProviderFailure::default);
            if let Some(id) = declared
                .request_id
                .as_deref()
                .and_then(|path| at(&event, path))
            {
                failure.request_id.get_or_insert(id);
            }
        } else if declared
            .carried_on
            .iter()
            .any(|shape| matches(&event, shape))
        {
            return found;
        }
    }

    found
}

/// One line of output as an event: a JSON object, or nothing. A CLI's diagnostics on stderr share
/// the transcript, and are not events.
fn event(line: &str) -> Option<Value> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Value>(line)
        .ok()
        .filter(Value::is_object)
}

/// Whether `event` has every field `shape` names, with the value it gives. A table in the shape
/// is matched against the object at that field the same way, so a shape says no more than it
/// must.
#[must_use]
pub fn matches(event: &Value, shape: &Shape) -> bool {
    shape
        .iter()
        .all(|(key, want)| match (event.get(key), want) {
            (Some(have @ Value::Object(_)), Value::Object(inner)) => matches(have, inner),
            (Some(have), want) => have == want,
            (None, _) => false,
        })
}

/// The value at a dotted path, as text.
fn at(event: &Value, path: &str) -> Option<String> {
    let value = path
        .split('.')
        .try_fold(event, |value, key| value.get(key))?;
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
