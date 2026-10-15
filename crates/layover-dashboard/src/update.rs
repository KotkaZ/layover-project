//! `GET /update`: what this machine knows about releases, for this factory.
//!
//! Read from the machine's own record of its last check rather than asked of a Tower, so a
//! `--watch-only` dashboard in another process says the same as the Tower's. Reading never
//! checks: the Tower does that on its schedule, and `layover update check` on request.

use layover_http::{AvailableUpdate, Problem, UpdateChannel, UpdateKind, UpdateMode, UpdateStatus};
use layover_update::{Channel, Kind, Mode, Settings, Version, compatibility, status};

use crate::api::Dashboard;

impl Dashboard {
    /// Where updates stand for this factory.
    pub(crate) fn update_status(&self) -> Result<UpdateStatus, Problem> {
        let config = self.config()?;
        let settings = self
            .update_settings()
            .cloned()
            .unwrap_or_else(Settings::from_env);
        let found = status(&settings, &config.update, &Version::running());
        let last = found.last.as_ref();

        Ok(UpdateStatus {
            current: found.running.to_string(),
            mode: match found.mode {
                Mode::Off => UpdateMode::Off,
                Mode::Notify => UpdateMode::Notify,
            },
            channel: match found.channel {
                Channel::Stable => UpdateChannel::Stable,
                Channel::Prerelease => UpdateChannel::Prerelease,
            },
            switched_off: found.switched_off,
            checked_at: last.map(|last| last.attempted_at.to_string()),
            succeeded_at: last.and_then(|last| last.succeeded_at.map(|at| at.to_string())),
            error: last.and_then(|last| last.error.clone()),
            latest: last
                .and_then(|last| last.newest.as_ref())
                .map(|newest| newest.version.to_string()),
            available: found.available.map(|available| {
                let release = available.release;
                AvailableUpdate {
                    version: release.version.to_string(),
                    kind: match available.kind {
                        Kind::Patch => UpdateKind::Patch,
                        Kind::Minor => UpdateKind::Minor,
                        Kind::Major => UpdateKind::Major,
                    },
                    prerelease: release.prerelease,
                    title: release.title,
                    published_at: release.published_at.map(|at| at.to_string()),
                    compatibility: compatibility(&release.notes),
                    notes: release.notes,
                    url: release.page,
                }
            }),
            skipped: found.skipped.iter().map(ToString::to_string).collect(),
        })
    }
}
