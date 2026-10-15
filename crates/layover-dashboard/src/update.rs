//! `GET /update`: what this machine knows about releases, for this factory.
//!
//! Read from the machine's own record of its last check rather than asked of a Tower, so a
//! `--watch-only` dashboard in another process says the same as the Tower's. Reading never
//! checks: the Tower does that on its schedule, and `layover update check` on request.

use std::path::PathBuf;

use axum::http::StatusCode;
use layover_http::{
    AvailableUpdate, Problem, StagedUpdate, UpdateChannel, UpdateGate, UpdateKind, UpdateMode,
    UpdateProvenance, UpdateRecord, UpdateRequestState, UpdateStatus,
};
use layover_update::handover::{self, Request};
use layover_update::{
    Attestation, Channel, Gate, Kind, Mode, Settings, Staged, Version, compatibility, status,
};

use crate::api::Dashboard;

impl Dashboard {
    fn settings(&self) -> Settings {
        self.update_settings()
            .cloned()
            .unwrap_or_else(Settings::from_env)
    }

    /// The factory's `.layover`, where the Tower looks for a request to install.
    fn update_state(&self) -> PathBuf {
        self.state()
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".layover")
    }

    /// Asks the Tower in this process to install the staged release.
    pub(crate) fn ask_to_install(&self, now: bool) -> Result<UpdateStatus, Problem> {
        let conflict = |title: &str, detail: String| {
            Problem::new(StatusCode::CONFLICT, title).with_detail(detail)
        };
        if !self.runs_a_tower() {
            return Err(conflict(
                "no Tower runs here",
                "This dashboard only watches. Install with `layover update apply` where the \
                 factory is served."
                    .to_owned(),
            ));
        }
        let current = self.update_status()?;
        if current
            .request
            .as_ref()
            .is_some_and(|request| request.outcome.is_none())
        {
            return Err(conflict(
                "an install is already under way",
                "Wait for it, or cancel it first.".to_owned(),
            ));
        }
        let (Some(available), Some(staged)) = (&current.available, &current.staged) else {
            return Err(conflict(
                "nothing is staged",
                "Stage it first: `layover update stage`, or `[update] mode = \"stage\"`."
                    .to_owned(),
            ));
        };
        if !staged.ready || staged.version != available.version {
            return Err(conflict(
                "the release is not staged and verified",
                staged.refused.clone().unwrap_or_else(|| {
                    "Its provenance could not be checked. `layover update apply \
                     --trust-checksum-only` installs it on its checksum alone."
                        .to_owned()
                }),
            ));
        }
        if staged.crosses_layout {
            return Err(conflict(
                "the release changes the state layout",
                "Install it by hand.".to_owned(),
            ));
        }
        if !current.gate.as_ref().is_some_and(|gate| gate.passes) {
            return Err(conflict(
                "the release refuses this factory, or has not been asked",
                "Stage it again to ask it: `layover update stage`.".to_owned(),
            ));
        }
        let version: Version = available
            .version
            .parse()
            .map_err(|_| conflict("not a version", available.version.clone()))?;
        self.put_request(version, false, now)
    }

    /// Asks the Tower in this process to go back to the kept previous binary.
    pub(crate) fn ask_to_roll_back(&self) -> Result<UpdateStatus, Problem> {
        if !self.runs_a_tower() {
            return Err(
                Problem::new(StatusCode::CONFLICT, "no Tower runs here").with_detail(
                    "Roll back with `layover update rollback` where the factory is served.",
                ),
            );
        }
        let running = Version::running();
        let Some(record) = handover::handover(&self.update_state())
            .filter(|record| record.to == running && record.previous.exists())
        else {
            return Err(
                Problem::new(StatusCode::CONFLICT, "nothing is kept to go back to")
                    .with_detail(format!("No earlier version is kept beside {running}.")),
            );
        };
        let _ = layover_update::state::skip(&self.settings().dir, &running);
        self.put_request(record.from, true, false)
    }

    fn put_request(
        &self,
        version: Version,
        rollback: bool,
        now: bool,
    ) -> Result<UpdateStatus, Problem> {
        handover::put_request(
            &self.update_state(),
            &Request {
                version,
                rollback,
                now,
                including_manual: false,
                by: "the dashboard".to_owned(),
                at: jiff::Timestamp::now(),
                progress: Vec::new(),
                outcome: None,
                until: None,
            },
        )
        .map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "could not ask the Tower")
                .with_detail(error.to_string())
        })?;
        self.update_status()
    }

    /// Cancels an install the Tower has not made yet.
    pub(crate) fn cancel_install(&self) -> Result<UpdateStatus, Problem> {
        let state = self.update_state();
        if handover::request(&state).is_some_and(|request| request.outcome.is_none()) {
            handover::progress(&state, "Cancelled from the dashboard.", Some("cancelled"));
        }
        self.update_status()
    }

    /// Stops offering `version` on this machine.
    pub(crate) fn skip_version(&self, version: &str) -> Result<UpdateStatus, Problem> {
        let version: Version = version.parse().map_err(|error| {
            Problem::new(StatusCode::BAD_REQUEST, "not a version").with_detail(format!("{error}"))
        })?;
        layover_update::state::skip(&self.settings().dir, &version).map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "could not skip it")
                .with_detail(error.to_string())
        })?;
        self.update_status()
    }

    /// Where updates stand for this factory.
    pub(crate) fn update_status(&self) -> Result<UpdateStatus, Problem> {
        let config = self.config()?;
        let settings = self.settings();
        let found = status(&settings, &config.update, &Version::running());
        let state = self.update_state();
        let record = handover::handover(&state);
        let last = found.last.as_ref();
        let staged = found
            .available
            .as_ref()
            .and_then(|available| layover_update::staged(&settings, &available.release.version));
        let gate = staged.as_ref().and_then(|staged| {
            layover_update::gate::recorded(&self.state().config_path)
                .filter(|gate| gate.version == staged.version)
        });

        Ok(UpdateStatus {
            current: found.running.to_string(),
            mode: match found.mode {
                Mode::Off => UpdateMode::Off,
                Mode::Notify => UpdateMode::Notify,
                Mode::Stage => UpdateMode::Stage,
                Mode::Auto => UpdateMode::Auto,
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
            staged: staged.as_ref().map(staged_update),
            gate: gate.as_ref().map(update_gate),
            request: handover::request(&state).map(|request| UpdateRequestState {
                version: request.version.to_string(),
                rollback: request.rollback,
                by: request.by,
                at: request.at.to_string(),
                progress: request.progress,
                outcome: request.outcome,
            }),
            previous: record
                .as_ref()
                .filter(|record| record.to == found.running && record.previous.exists())
                .map(|record| record.from.to_string()),
            last_update: record.map(|record| UpdateRecord {
                from: record.from.to_string(),
                to: record.to.to_string(),
                started: record.started.to_string(),
                healthy_at: record.healthy_at.map(|at| at.to_string()),
                rolled_back_at: record.rolled_back_at.map(|at| at.to_string()),
            }),
        })
    }
}

fn staged_update(staged: &Staged) -> StagedUpdate {
    let (provenance, detail) = match &staged.attestation {
        Some(Attestation::Verified { signer }) => (UpdateProvenance::Verified, Some(signer)),
        Some(Attestation::Unavailable { reason }) => (UpdateProvenance::Unavailable, Some(reason)),
        Some(Attestation::Refused { reason }) => (UpdateProvenance::Refused, Some(reason)),
        None => (UpdateProvenance::Unchecked, None),
    };
    StagedUpdate {
        version: staged.version.to_string(),
        at: Some(staged.at.to_string()),
        archive: (!staged.archive.is_empty()).then(|| staged.archive.clone()),
        sha256: staged.sha256.clone(),
        provenance,
        provenance_detail: detail.cloned(),
        trusted_checksum_only: staged.trusted_checksum_only,
        ready: staged.is_ready(),
        crosses_layout: staged.crosses_layout,
        refused: staged.refused.clone(),
    }
}

fn update_gate(gate: &Gate) -> UpdateGate {
    UpdateGate {
        version: gate.version.to_string(),
        at: Some(gate.at.to_string()),
        not_run: gate.not_run.clone(),
        passes: gate.passes(),
        passes_unattended: gate.passes_unattended(),
        errors: gate.errors.clone(),
        new_warnings: gate.new_warnings.clone(),
    }
}
