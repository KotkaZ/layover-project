//! Day-segmented JSON Lines files, shared by everything that keeps history.
//!
//! Run records and help requests are different things asked different questions, but they are
//! stored identically: one JSON object per line, in a file named for the UTC day it covers. The
//! mechanics live here once so the two cannot drift apart — a bug fixed in one reader and not the
//! other is the kind that stays hidden for months.
//!
//! See [`crate`] for why the segments are UTC days while the windows that query them are local.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use layover_core::cost::Span;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::history::StoreError;

/// Appends one line to a segment, creating it if needed.
///
/// Opened, written and closed per line. That is slower than holding the handle open and it is the
/// right trade: a factory writes a handful of records a minute, and a file that is never held
/// open cannot be left truncated by a process that dies mid-run — which, given that recording
/// interruptions is the whole point, is not a hypothetical.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the line cannot be written.
pub fn append_line(path: &Path, line: &str) -> Result<(), StoreError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(StoreError::at(path))?;
    writeln!(file, "{line}").map_err(StoreError::at(path))
}

/// Reads one segment, skipping lines that will not parse.
///
/// A truncated final line is what a power cut leaves behind, and losing the rest of the file to it
/// would turn a crash into data loss. Blank lines are skipped for the same reason.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the file exists but cannot be read.
pub fn read_segment<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, StoreError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(StoreError::at(path)(error)),
    };

    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(StoreError::at(path))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<T>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Reads every segment that could hold a record inside `span`.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the directory cannot be listed or a segment cannot be read.
pub fn read_segments<T: DeserializeOwned>(
    root: &Path,
    prefix: &str,
    span: &Span,
) -> Result<Vec<T>, StoreError> {
    let mut found = Vec::new();
    for path in segments_covering(root, prefix, span)? {
        found.extend(read_segment::<T>(&path)?);
    }
    Ok(found)
}

/// Replaces a segment's contents.
///
/// Written to a neighbouring file and renamed, so an interrupted rewrite cannot destroy a day of
/// history. Appending is safe to do in place because a torn line costs one record; rewriting is
/// not, because a torn rewrite costs the file.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if it cannot be written or moved into place, or
/// [`StoreError::Serialise`] if a record cannot be encoded.
pub fn rewrite_segment<T: Serialize>(path: &Path, records: &[T]) -> Result<(), StoreError> {
    let staging = path.with_extension("jsonl.writing");

    let mut body = String::new();
    for record in records {
        body.push_str(&serde_json::to_string(record)?);
        body.push('\n');
    }

    fs::write(&staging, body).map_err(StoreError::at(&staging))?;
    fs::rename(&staging, path).map_err(StoreError::at(path))
}

/// Deletes segments that have fallen past the retention horizon, returning how many went.
///
/// Whole files only. A segment is kept until every record it could hold is expired, so a little
/// more than the horizon survives — erring towards keeping data rather than towards deleting
/// something still inside the window.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the directory cannot be listed or a file cannot be removed.
pub fn prune_segments(root: &Path, prefix: &str, horizon: Timestamp) -> Result<usize, StoreError> {
    let cutoff = horizon
        .to_zoned(TimeZone::UTC)
        .date()
        .checked_sub(1.day())
        .unwrap_or(Date::MIN);

    let mut removed = 0;
    for (date, path) in segments(root, prefix)? {
        if date < cutoff {
            fs::remove_file(&path).map_err(StoreError::at(&path))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// The segment a record timestamped `at` belongs in.
pub fn segment_for(root: &Path, prefix: &str, at: Timestamp) -> PathBuf {
    let date = at.to_zoned(TimeZone::UTC).date();
    root.join(format!("{prefix}-{date}.jsonl"))
}

/// Every segment with this prefix, oldest first, paired with the day it holds.
///
/// Anything not named like a segment is ignored rather than reported: the directory is a user's to
/// look in, and an editor's stray `.swp` file is not a corruption.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the directory exists but cannot be listed.
pub fn segments(root: &Path, prefix: &str) -> Result<Vec<(Date, PathBuf)>, StoreError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(StoreError::at(root)(error)),
    };

    let mut found = std::collections::BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(StoreError::at(root))?;
        let path = entry.path();
        if let Some(date) = segment_date(&path, prefix) {
            found.insert((date, path));
        }
    }
    Ok(found.into_iter().collect())
}

/// The segments that could hold a record inside `span`.
///
/// One UTC day either side of the window, because a local day overlaps two UTC ones at every
/// offset but zero. Cheap insurance: filtering by instant is what actually decides membership.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the directory cannot be listed.
pub fn segments_covering(
    root: &Path,
    prefix: &str,
    span: &Span,
) -> Result<Vec<PathBuf>, StoreError> {
    let first = span.start.map(|start| {
        start
            .to_zoned(TimeZone::UTC)
            .date()
            .checked_sub(1.day())
            .unwrap_or(Date::MIN)
    });
    let last = span
        .end
        .to_zoned(TimeZone::UTC)
        .date()
        .checked_add(1.day())
        .unwrap_or(Date::MAX);

    Ok(segments(root, prefix)?
        .into_iter()
        .filter(|(date, _)| first.is_none_or(|first| *date >= first) && *date <= last)
        .map(|(_, path)| path)
        .collect())
}

/// Extracts the day from a segment filename, or `None` if it is not one.
fn segment_date(path: &Path, prefix: &str) -> Option<Date> {
    path.file_name()?
        .to_str()?
        .strip_prefix(prefix)?
        .strip_prefix('-')?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

/// Reads a whole-document store: one JSON object per line, all of it one record.
///
/// Distinct from a segment despite the shape. A segment holds events, so a torn line costs one
/// event; here the file is the record, which is why the writer is atomic.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if the file exists but cannot be read.
pub fn read_document<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, StoreError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(StoreError::at(path)(error)),
    };

    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Writes a whole-document store, atomically.
///
/// Written to a neighbouring file and renamed, so an interrupted write cannot leave the factory
/// with half its memory.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if it cannot be written or moved into place, or
/// [`StoreError::Serialise`] if a record cannot be encoded.
pub fn write_document<T: Serialize>(path: &Path, records: &[T]) -> Result<(), StoreError> {
    let staging = path.with_extension("jsonl.writing");

    let mut body = String::new();
    for record in records {
        body.push_str(&serde_json::to_string(record)?);
        body.push('\n');
    }

    fs::write(&staging, body).map_err(StoreError::at(&staging))?;
    fs::rename(&staging, path).map_err(StoreError::at(path))
}
