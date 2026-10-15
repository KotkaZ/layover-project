//! `GET /update`: what this machine knows about releases, for this factory.
//!
//! Read from the machine's own record of its last check rather than asked of a Tower, so a
//! `--watch-only` dashboard in another process says the same as the Tower's. Reading never
//! checks: the Tower does that on its schedule, and `layover update check` on request.

use layover_http::{
    AvailableUpdate, Problem, StagedUpdate, UpdateChannel, UpdateGate, UpdateKind, UpdateMode,
    UpdateProvenance, UpdateStatus,
};
use layover_update::{
    Attestation, Channel, Gate, Kind, Mode, Settings, Staged, Version, compatibility, status,
};

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
