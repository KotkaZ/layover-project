//! `layover update stage`, and what the Tower does under `[update] mode = "stage"`: get a newer
//! release ready — downloaded, verified, asked what it is, and measured against this factory —
//! without installing it.

use std::fmt::Write as _;
use std::path::Path;

use layover_core::{Config, PromptDir, validate, validate_prompts};
use layover_update::stage::Running;
use layover_update::{Attestation, Gate, Gh, Release, Settings, Staged, Version, gate, stage};

use crate::commands::Failure;

/// What is running, for a staged binary to be measured against.
pub fn running() -> Running {
    Running {
        version: Version::running(),
        layout: layover_store::layout::CURRENT,
    }
}

/// The running version's own warnings for the factory at `path`, worded as `validate` prints
/// them: what a staged release's warnings are measured against.
pub fn warnings_of(path: &Path) -> Vec<String> {
    let Ok(config) = Config::load(path) else {
        return Vec::new();
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let source = PromptDir::new(base.join(&config.layover.prompt_dir));
    let mut found = validate(&config);
    found.extend(validate_prompts(&config, &source));
    found
        .into_iter()
        .filter(|diagnostic| !diagnostic.is_error())
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Stages `release` and gates it against the factory at `factory`, recording both.
pub fn stage_and_gate(
    settings: &Settings,
    release: &Release,
    factory: &Path,
    trust_checksum_only: bool,
) -> (Staged, Gate) {
    let staged = stage(
        settings,
        release,
        &running(),
        &Gh::default(),
        trust_checksum_only,
    );
    let gate = gate(&staged, factory, &warnings_of(factory));
    let _ = layover_update::gate::record(&gate);
    (staged, gate)
}

/// Gets `release` ready for the factory at `factory`, as the Tower does after a check under
/// `[update] mode = "stage"`: staged, unless it already is — perhaps by another factory's Tower on
/// this machine — and gated against this factory, unless it already was. A release that stopped
/// short last time is tried again: a download can break off, and `gh` can be signed in since.
pub fn ready(settings: &Settings, release: &Release, factory: &Path) -> Option<(Staged, Gate)> {
    let staged = layover_update::staged(settings, &release.version);
    match staged {
        Some(staged) if staged.is_ready() => {
            let gated = layover_update::gate::recorded(factory)
                .is_some_and(|gate| gate.version == release.version);
            if gated {
                return None;
            }
            let gate = gate(&staged, factory, &warnings_of(factory));
            let _ = layover_update::gate::record(&gate);
            Some((staged, gate))
        }
        _ => Some(stage_and_gate(settings, release, factory, false)),
    }
}

/// `layover update stage`: looks, and gets the newest release ready for this factory.
///
/// # Errors
///
/// Returns why nothing could be staged: switched off, offline, nothing newer, or refused.
pub fn stage_now(path: &Path, trust_checksum_only: bool) -> Result<String, Failure> {
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let config = Config::load(&path).map_err(|error| error.to_string())?;
    let settings = Settings::from_env();
    layover_update::check(&settings, config.update.channel, jiff::Timestamp::now())
        .map_err(|error| error.to_string())?;
    let status = layover_update::status(&settings, &config.update, &Version::running());
    let Some(available) = status.available else {
        return Ok("This is the newest release: there is nothing to stage.\n".to_owned());
    };
    let (staged, gate) = stage_and_gate(&settings, &available.release, &path, trust_checksum_only);
    let said = describe(&staged, Some(&gate));
    if staged.refused.is_some() || !gate.passes() && staged.is_ready() {
        Err(said)
    } else {
        Ok(said)
    }
}

/// Says what staging found, a line a step, for a person.
pub fn describe(staged: &Staged, gate: Option<&Gate>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Layover {} for {}:",
        staged.version,
        layover_update::TARGET
    );
    if let Some(digest) = &staged.sha256 {
        let _ = writeln!(
            out,
            "  {}: SHA-256 {digest} matches the release.",
            staged.archive
        );
    }
    match &staged.attestation {
        Some(Attestation::Verified { signer }) => {
            let _ = writeln!(out, "  Provenance verified: built by {signer}.");
        }
        Some(Attestation::Unavailable { reason }) => {
            let _ = writeln!(out, "  Provenance not verified: {reason}.");
            if staged.trusted_checksum_only {
                out.push_str("  Run anyway, as --trust-checksum-only says.\n");
            } else {
                out.push_str(
                    "  So it was not run. Sign gh in and stage again, or say \
                     --trust-checksum-only to run it on its checksum alone.\n",
                );
            }
        }
        Some(Attestation::Refused { reason }) => {
            let _ = writeln!(out, "  Provenance refused: {reason}.");
        }
        None => {}
    }
    if let Some(about) = &staged.about {
        let _ = writeln!(
            out,
            "  It says it is {} for {}, state layout {}{}.",
            about.version,
            about.target,
            about.layout,
            if staged.crosses_layout {
                " — a different layout from this one, so installing it is a person's choice"
            } else {
                ", the same as this one"
            }
        );
    }
    if let Some(refused) = &staged.refused {
        let _ = writeln!(out, "Refused: {refused}.");
        return out;
    }
    if let Some(gate) = gate {
        describe_gate(gate, &mut out);
    }
    out
}

fn describe_gate(gate: &Gate, out: &mut String) {
    let factory = gate.factory.display();
    if let Some(why) = &gate.not_run {
        let _ = writeln!(out, "  Not yet checked against {factory}: {why}.");
        return;
    }
    if !gate.errors.is_empty() {
        let _ = writeln!(out, "  It refuses {factory}:");
        for error in &gate.errors {
            let _ = writeln!(out, "    error: {error}");
        }
        return;
    }
    if gate.new_warnings.is_empty() {
        let _ = writeln!(
            out,
            "  It validates {factory}, with no new warnings. Ready to install."
        );
    } else {
        let _ = writeln!(
            out,
            "  It validates {factory}, with warnings this version does not give — so it would not \
             be installed unattended:"
        );
        for warning in &gate.new_warnings {
            let _ = writeln!(out, "    warning: {warning}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_factorys_warnings_are_its_own_validation() {
        let dir =
            std::env::temp_dir().join(format!("layover-staging-warnings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates");
        let path = dir.join("layover.toml");
        std::fs::write(
            &path,
            "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n",
        )
        .expect("writes");

        let warnings = warnings_of(&path);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("description")),
            "{warnings:?}"
        );
        assert!(warnings_of(&dir.join("missing.toml")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
