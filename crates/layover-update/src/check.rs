//! Looking for a newer release, how often, and what to say about one.
//!
//! A check is one request for a public file. It never fails anything else: offline, behind a proxy
//! that refuses it, or answered with an error, the Tower carries on and `doctor` says when a check
//! last worked. Failures back off — an hour, then two, four and so on up to the factory's
//! `check_every` — and a success waits `check_every` give or take a twelfth, so machines that
//! started together do not keep asking in the same minute.

use std::path::PathBuf;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use crate::source::{Fetched, Source, SourceError};
use crate::state::{self, LastCheck, Release};
use crate::version::{Kind, Version};
use crate::{Channel, Mode, UpdateConfig};

/// What the updater is allowed to do on this machine, and where it looks and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `LAYOVER_UPDATE=off`: no request is made, whatever a factory says.
    pub switched_off: bool,
    /// Where releases come from.
    pub source: Source,
    /// Where this machine keeps what it knows about them.
    pub dir: PathBuf,
}

impl Settings {
    /// What the environment says: `LAYOVER_UPDATE`, `LAYOVER_UPDATE_SOURCE` and
    /// `LAYOVER_GITHUB_TOKEN`, and the platform's state directory.
    #[must_use]
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let source = match var("LAYOVER_UPDATE_SOURCE") {
            Some(base) => Source::mirror(&base),
            None => Source::github(var("LAYOVER_GITHUB_TOKEN")),
        };
        Self {
            switched_off: var("LAYOVER_UPDATE")
                .is_some_and(|value| value.eq_ignore_ascii_case("off")),
            source,
            dir: state::machine_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("layover").join("updates")),
        }
    }
}

/// Why a check found nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// `LAYOVER_UPDATE=off`.
    #[error("update checks are switched off on this machine (LAYOVER_UPDATE=off)")]
    SwitchedOff,
    /// The source could not be reached or read.
    #[error(transparent)]
    Source(#[from] SourceError),
}

/// Looks for the newest release on `channel` now, and records what was found or why nothing was.
///
/// # Errors
///
/// Returns [`CheckError::SwitchedOff`] without making a request when `LAYOVER_UPDATE=off`, and
/// the source's error when it could not be reached or read.
pub fn check(settings: &Settings, channel: Channel, now: Timestamp) -> Result<Release, CheckError> {
    if settings.switched_off {
        return Err(CheckError::SwitchedOff);
    }
    let last = state::last_check(&settings.dir, channel);
    let known = last.as_ref().and_then(|last| last.newest.as_ref());
    // An ETag is only worth sending when what it stands for is still on record.
    let etag = last
        .as_ref()
        .filter(|_| known.is_some())
        .and_then(|last| last.etag.as_ref());

    let fetched = settings.source.newest(channel, etag);
    let (release, etag) = match fetched {
        Ok(Fetched::Found(found)) => {
            let version = found.manifest.version().unwrap_or_else(Version::running);
            let tag = found
                .manifest
                .announcement_tag
                .clone()
                .unwrap_or_else(|| format!("v{version}"));
            let release = Release {
                page: settings.source.page_for(&tag),
                prerelease: found.manifest.announcement_is_prerelease || version.is_prerelease(),
                version,
                tag,
                title: found.manifest.announcement_title.clone(),
                notes: found
                    .manifest
                    .announcement_changelog
                    .clone()
                    .unwrap_or_default(),
                published_at: found.published_at.or_else(|| {
                    let day = found.manifest.released_on()?;
                    day.to_zoned(jiff::tz::TimeZone::UTC)
                        .ok()
                        .map(|at| at.timestamp())
                }),
            };
            (release, found.etag)
        }
        Ok(Fetched::Unchanged) => match known {
            Some(release) => (
                release.clone(),
                last.as_ref().and_then(|last| last.etag.clone()),
            ),
            None => unreachable!("an ETag is sent only with a release on record"),
        },
        Err(error) => {
            let failed = LastCheck {
                channel,
                attempted_at: now,
                succeeded_at: last.as_ref().and_then(|last| last.succeeded_at),
                error: Some(error.to_string()),
                failures: last
                    .as_ref()
                    .map_or(0, |last| last.failures)
                    .saturating_add(1),
                newest: known.cloned(),
                etag: last.as_ref().and_then(|last| last.etag.clone()),
            };
            let _ = state::record(&settings.dir, &failed);
            return Err(error.into());
        }
    };

    let _ = state::record(
        &settings.dir,
        &LastCheck {
            channel,
            attempted_at: now,
            succeeded_at: Some(now),
            error: None,
            failures: 0,
            newest: Some(release.clone()),
            etag,
        },
    );
    Ok(release)
}

/// When the next scheduled check is due, after `last`: now, if there has been none.
#[must_use]
pub fn next_check(last: Option<&LastCheck>, every: Duration) -> Option<Timestamp> {
    let last = last?;
    let every = SignedDuration::try_from(every).unwrap_or(SignedDuration::MAX);
    let wait = if last.failures > 0 {
        let doublings = last.failures.saturating_sub(1).min(16);
        SignedDuration::from_hours(1 << doublings).min(every)
    } else {
        every.saturating_add(jitter(last.attempted_at, every))
    };
    last.attempted_at.checked_add(wait).ok()
}

/// Whether a scheduled check is due at `now`.
#[must_use]
pub fn due(last: Option<&LastCheck>, every: Duration, now: Timestamp) -> bool {
    next_check(last, every).is_none_or(|next| next <= now)
}

/// Up to a twelfth of `every` either way — two hours of a day — chosen by when the last check was.
fn jitter(seed: Timestamp, every: SignedDuration) -> SignedDuration {
    let span = every.as_secs() / 12;
    if span <= 0 {
        return SignedDuration::ZERO;
    }
    let width = u128::from(span.saturating_mul(2).saturating_add(1).unsigned_abs());
    let offset = i64::try_from(seed.as_nanosecond().unsigned_abs() % width).unwrap_or(0);
    SignedDuration::from_secs(offset - span)
}

/// A release newer than the running one, and how far a move to it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    /// The release.
    pub release: Release,
    /// Patch, minor or major.
    pub kind: Kind,
}

/// The release in `newest`, when it is newer than `running`, not skipped, and on `channel`.
#[must_use]
pub fn available(
    running: &Version,
    newest: Option<&Release>,
    channel: Channel,
    skipped: &[Version],
) -> Option<Available> {
    let release = newest?;
    let wanted = release.version > *running
        && !skipped.contains(&release.version)
        && (channel == Channel::Prerelease || !release.prerelease);
    wanted.then(|| Available {
        release: release.clone(),
        kind: running.kind_of_move_to(&release.version),
    })
}

/// Where updates stand for one factory on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The version running.
    pub running: Version,
    /// The factory's mode.
    pub mode: Mode,
    /// The factory's channel.
    pub channel: Channel,
    /// Whether `LAYOVER_UPDATE=off` stops every check on this machine.
    pub switched_off: bool,
    /// The last check on the factory's channel.
    pub last: Option<LastCheck>,
    /// A newer release, if the last check that worked found one.
    pub available: Option<Available>,
    /// Versions a person said to skip.
    pub skipped: Vec<Version>,
}

/// Where updates stand for a factory configured with `config`, running `running`.
#[must_use]
pub fn status(settings: &Settings, config: &UpdateConfig, running: &Version) -> Status {
    let last = state::last_check(&settings.dir, config.channel);
    let skipped = state::skipped(&settings.dir);
    let available = available(
        running,
        last.as_ref().and_then(|last| last.newest.as_ref()),
        config.channel,
        &skipped,
    );
    Status {
        running: running.clone(),
        mode: config.mode,
        channel: config.channel,
        switched_off: settings.switched_off,
        last,
        available,
        skipped,
    }
}

/// The paragraph of release notes that says what an existing factory will notice: from
/// `**Compatibility.**` to the first blank line after it.
#[must_use]
pub fn compatibility(notes: &str) -> Option<String> {
    let start = notes.find("**Compatibility.**")?;
    let rest = &notes[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    Some(rest[..end].trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("valid")
    }

    fn checked(at_: &str, failures: u32) -> LastCheck {
        LastCheck {
            channel: Channel::Stable,
            attempted_at: at(at_),
            succeeded_at: None,
            error: None,
            failures,
            newest: None,
            etag: None,
        }
    }

    fn release(version: &str) -> Release {
        let version: Version = version.parse().expect("valid");
        Release {
            prerelease: version.is_prerelease(),
            tag: format!("v{version}"),
            version,
            title: None,
            notes: String::new(),
            published_at: None,
            page: String::new(),
        }
    }

    #[test]
    fn a_check_is_due_at_once_then_about_a_day_later() {
        let day = Duration::from_hours(24);
        assert!(due(None, day, at("2026-10-15T09:00:00Z")));

        let last = checked("2026-10-15T09:00:00Z", 0);
        assert!(!due(Some(&last), day, at("2026-10-16T06:59:59Z")));
        assert!(due(Some(&last), day, at("2026-10-16T11:00:00Z")));
        let next = next_check(Some(&last), day).expect("scheduled");
        assert!(next >= at("2026-10-16T07:00:00Z") && next <= at("2026-10-16T11:00:00Z"));
    }

    #[test]
    fn a_failed_check_is_retried_sooner_and_backs_off_to_the_interval() {
        let day = Duration::from_hours(24);
        let after = |failures| {
            let last = checked("2026-10-15T09:00:00Z", failures);
            next_check(Some(&last), day)
                .expect("scheduled")
                .duration_since(at("2026-10-15T09:00:00Z"))
        };
        assert_eq!(after(1), SignedDuration::from_hours(1));
        assert_eq!(after(2), SignedDuration::from_hours(2));
        assert_eq!(after(5), SignedDuration::from_hours(16));
        assert_eq!(after(9), SignedDuration::from_hours(24));
    }

    #[test]
    fn only_a_newer_release_on_the_channel_that_is_not_skipped_is_available() {
        let running: Version = "1.14.0".parse().expect("valid");
        let newer = release("1.15.0");
        let found = available(&running, Some(&newer), Channel::Stable, &[]).expect("found");
        assert_eq!(found.kind, Kind::Minor);

        assert!(available(&running, Some(&release("1.14.0")), Channel::Stable, &[]).is_none());
        assert!(available(&running, Some(&release("1.13.2")), Channel::Stable, &[]).is_none());
        assert!(
            available(
                &running,
                Some(&newer),
                Channel::Stable,
                std::slice::from_ref(&newer.version)
            )
            .is_none()
        );

        let candidate = release("1.15.0-rc.1");
        assert!(available(&running, Some(&candidate), Channel::Stable, &[]).is_none());
        assert!(available(&running, Some(&candidate), Channel::Prerelease, &[]).is_some());
    }

    #[test]
    fn the_compatibility_paragraph_is_found_in_release_notes() {
        let notes = "Summary.\n\n**Compatibility.** Additive.\n- one\n- two\n\n### Added\n";
        assert_eq!(
            compatibility(notes).as_deref(),
            Some("**Compatibility.** Additive.\n- one\n- two")
        );
        assert_eq!(compatibility("nothing here"), None);
    }
}
