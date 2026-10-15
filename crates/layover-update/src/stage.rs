//! Staging a release: its archive downloaded, checked against the release, its provenance
//! verified, and its binary asked what it is — everything short of installing it.
//!
//! In order, each step only if the one before it passed:
//!
//! 1. the release's manifest names an archive for this platform;
//! 2. the archive's SHA-256 equals the manifest's and the `.sha256` file's;
//! 3. its attestation says Layover's release workflow built it from the release's tag;
//! 4. it holds the binary and nothing that would land outside where it is unpacked;
//! 5. the binary, run for the first time, says it is the version and platform it should be, and
//!    that it can be reached from the version running.
//!
//! Nothing downloaded runs before step 5, and step 5 runs only after step 3 verified it — or a
//! person said to trust the checksum alone. A refusal at any step is recorded with its reason, and
//! the archive is deleted. What was staged is kept per machine, one version at a time, in
//! `staging/<version>/` under the update directory.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::Settings;
use crate::download::{MAX_ARCHIVE, digest_of, fetch, sidecar};
use crate::probe::{About, about};
use crate::provenance::{Attestation, Verifier};
use crate::state::Release;
use crate::version::Version;

/// What is running, to measure a staged binary against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    /// Its version.
    pub version: Version,
    /// The state-directory layout it reads and writes.
    pub layout: u32,
}

/// A release, staged or refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staged {
    /// Its version.
    pub version: Version,
    /// Its tag.
    pub tag: String,
    /// When staging finished.
    pub at: Timestamp,
    /// The archive's name, and its SHA-256 once it matched the release.
    pub archive: String,
    /// The archive's SHA-256, when it was downloaded and matched.
    #[serde(default)]
    pub sha256: Option<String>,
    /// What its attestation said, when it was checked.
    #[serde(default)]
    pub attestation: Option<Attestation>,
    /// Whether a person said to run it with its checksum alone.
    #[serde(default)]
    pub trusted_checksum_only: bool,
    /// The unpacked binary, when it got that far.
    #[serde(default)]
    pub binary: Option<PathBuf>,
    /// What the binary said it is, when it was run.
    #[serde(default)]
    pub about: Option<About>,
    /// Whether it reads a different state-directory layout from the one running: an update a
    /// person should choose, never one applied unattended.
    #[serde(default)]
    pub crosses_layout: bool,
    /// Why staging stopped short, if it did.
    #[serde(default)]
    pub refused: Option<String>,
}

impl Staged {
    /// Whether it is downloaded, verified and has answered for itself: ready to install.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.refused.is_none()
            && self.about.is_some()
            && self
                .attestation
                .as_ref()
                .is_some_and(|found| found.is_verified() || self.trusted_checksum_only)
    }
}

/// Where release `version` is staged.
#[must_use]
pub fn dir_for(settings: &Settings, version: &Version) -> PathBuf {
    settings.dir.join("staging").join(version.to_string())
}

/// The release staged for `version`, if one was.
#[must_use]
pub fn staged(settings: &Settings, version: &Version) -> Option<Staged> {
    let text = std::fs::read_to_string(dir_for(settings, version).join("staged.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// The binary's file name inside a release, per platform.
fn binary_name() -> &'static str {
    if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    }
}

/// Stages `release` for the platform this binary was built for.
///
/// `trust_checksum_only` lets a person run a binary whose provenance could not be checked; it
/// never overrides a refusal.
pub fn stage(
    settings: &Settings,
    release: &Release,
    running: &Running,
    verifier: &dyn Verifier,
    trust_checksum_only: bool,
) -> Staged {
    let dir = dir_for(settings, &release.version);
    let mut staged = Staged {
        version: release.version.clone(),
        tag: release.tag.clone(),
        at: Timestamp::now(),
        archive: String::new(),
        sha256: None,
        attestation: None,
        trusted_checksum_only: trust_checksum_only,
        binary: None,
        about: None,
        crosses_layout: false,
        refused: None,
    };
    if let Err(refused) = steps(settings, release, running, verifier, &dir, &mut staged) {
        staged.refused = Some(refused);
        staged.binary = None;
        let _ = std::fs::remove_file(dir.join(&staged.archive));
        let _ = std::fs::remove_file(dir.join(binary_name()));
    }
    staged.at = Timestamp::now();
    let _ = record(&dir, &staged);
    forget_others(settings, &release.version);
    staged
}

fn steps(
    settings: &Settings,
    release: &Release,
    running: &Running,
    verifier: &dyn Verifier,
    dir: &Path,
    staged: &mut Staged,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("could not stage it: {error}"))?;

    // 1. What the release carries for this platform.
    let manifest = settings
        .source
        .manifest_of(&release.tag)
        .map_err(|error| error.to_string())?;
    let (name, artifact) = manifest
        .archive_for(crate::TARGET)
        .ok_or_else(|| format!("{} has no archive for {}", release.version, crate::TARGET))?;
    name.clone_into(&mut staged.archive);
    let listed = artifact
        .checksums
        .get("sha256")
        .map(|digest| digest.to_ascii_lowercase())
        .ok_or_else(|| format!("the release lists no SHA-256 for {name}"))?;
    let executable = artifact
        .assets
        .iter()
        .find(|asset| asset.kind.as_deref() == Some("executable"))
        .and_then(|asset| asset.path.clone().or_else(|| asset.name.clone()))
        .unwrap_or_else(|| binary_name().to_owned());

    // 2. The archive, and that it is the one listed.
    let archive = dir.join(name);
    let downloaded = if archive.exists() && digest_of(&archive).ok().as_deref() == Some(&listed) {
        listed.clone()
    } else {
        fetch(
            &settings.source.file_in(&release.tag, name),
            &archive,
            MAX_ARCHIVE,
        )?
    };
    if downloaded != listed {
        return Err(format!(
            "the archive's SHA-256 is {downloaded}, but the release lists {listed}"
        ));
    }
    let beside = sidecar(
        &settings
            .source
            .file_in(&release.tag, &format!("{name}.sha256")),
    )?;
    if beside != listed {
        return Err(format!(
            "the release's {name}.sha256 says {beside}, but its manifest says {listed}"
        ));
    }
    staged.sha256 = Some(listed);

    // 3. Who built it.
    let attestation = verifier.verify(&archive, &release.tag);
    staged.attestation = Some(attestation.clone());
    match attestation {
        Attestation::Refused { reason } => {
            return Err(format!("its provenance did not verify: {reason}"));
        }
        Attestation::Unavailable { .. } if !staged.trusted_checksum_only => {
            // Kept, and not run: a person may apply it with --trust-checksum-only.
            return Ok(());
        }
        Attestation::Verified { .. } | Attestation::Unavailable { .. } => {}
    }

    // 4. The binary, and nothing else.
    let binary = dir.join(binary_name());
    crate::unpack::executable(&archive, &executable, &binary)
        .map_err(|why| format!("the archive was refused: {why}"))?;
    staged.binary = Some(binary.clone());

    // 5. The first time anything downloaded runs.
    let said = about(&binary, dir).map_err(|why| format!("it did not say what it is: {why}"))?;
    if said.version != release.version.to_string() {
        return Err(format!(
            "it says it is {}, but the release is {}",
            said.version, release.version
        ));
    }
    if said.target != crate::TARGET {
        return Err(format!(
            "it was built for {}, not {}",
            said.target,
            crate::TARGET
        ));
    }
    let oldest: Version = said
        .min_upgrade_from
        .parse()
        .map_err(|error| format!("it names no version it updates from: {error}"))?;
    if running.version < oldest {
        return Err(format!(
            "it cannot be reached from {} in one step; install {oldest} or later first",
            running.version
        ));
    }
    staged.crosses_layout = said.layout != running.layout;
    staged.about = Some(said);
    Ok(())
}

fn record(dir: &Path, staged: &Staged) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(staged).map_err(std::io::Error::other)?;
    let partial = dir.join(format!("staged.json.partial-{}", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(partial, dir.join("staged.json"))
}

/// Removes every staged version but `keep`: one is all that is ever installed next.
fn forget_others(settings: &Settings, keep: &Version) {
    let Ok(entries) = std::fs::read_dir(settings.dir.join("staging")) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() != keep.to_string() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Removes every staged version no newer than `running`: what was installed, or passed by.
pub fn forget_installed(settings: &Settings, running: &Version) {
    let Ok(entries) = std::fs::read_dir(settings.dir.join("staging")) else {
        return;
    };
    for entry in entries.flatten() {
        let older = entry
            .file_name()
            .to_string_lossy()
            .parse::<Version>()
            .map_or(true, |version| version <= *running);
        if older {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
