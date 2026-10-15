//! The release manifest `dist` publishes beside every release, `dist-manifest.json`.
//!
//! It says what a release is — its version, whether it is a pre-release, its notes — and what it
//! carries: an archive per platform with its SHA-256. Only the fields an update reads are parsed,
//! and every one is optional or defaulted, so a later `dist` adding fields changes nothing here.
//! A test against a pinned copy of a real manifest says when one it renames would.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::version::Version;

/// The name `dist` gives the application in a manifest: the crate's, not the binary's.
pub const APP_NAME: &str = "layover-cli";

/// A release manifest.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Manifest {
    /// The tag the release was cut from, such as `v1.14.0`.
    #[serde(default)]
    pub announcement_tag: Option<String>,
    /// Whether GitHub marks the release a pre-release.
    #[serde(default)]
    pub announcement_is_prerelease: bool,
    /// The release's title: `1.14.0 — 2026-10-14`.
    #[serde(default)]
    pub announcement_title: Option<String>,
    /// That release's section of the CHANGELOG, Compatibility first.
    #[serde(default)]
    pub announcement_changelog: Option<String>,
    /// Each application the release carries, and its version.
    #[serde(default)]
    pub releases: Vec<Application>,
    /// Every file the release carries, by name.
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
}

/// One application in a release.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Application {
    /// The crate's name.
    #[serde(default)]
    pub app_name: String,
    /// Its version.
    #[serde(default)]
    pub app_version: String,
}

/// One file a release carries.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Artifact {
    /// What it is: `executable-zip` for an archive holding the binary.
    #[serde(default)]
    pub kind: Option<String>,
    /// The platforms it is for.
    #[serde(default)]
    pub target_triples: Vec<String>,
    /// Its checksums, by algorithm.
    #[serde(default)]
    pub checksums: BTreeMap<String, String>,
    /// What is inside an archive.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// One file inside an archive.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Asset {
    /// Its name.
    #[serde(default)]
    pub name: Option<String>,
    /// Its path inside the archive.
    #[serde(default)]
    pub path: Option<String>,
    /// What it is: `executable` for the binary.
    #[serde(default)]
    pub kind: Option<String>,
}

impl Manifest {
    /// Parses a manifest.
    ///
    /// # Errors
    ///
    /// Returns the JSON error when the text is not a manifest.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The version of Layover this release carries: its application's, or else its tag's.
    #[must_use]
    pub fn version(&self) -> Option<Version> {
        self.releases
            .iter()
            .find(|release| release.app_name == APP_NAME)
            .and_then(|release| release.app_version.parse().ok())
            .or_else(|| self.announcement_tag.as_deref()?.parse().ok())
    }

    /// The day the release was cut, from its title.
    #[must_use]
    pub fn released_on(&self) -> Option<jiff::civil::Date> {
        self.announcement_title
            .as_deref()?
            .split_whitespace()
            .find_map(|word| word.parse().ok())
    }

    /// The archive holding the binary for `target`, and its name.
    #[must_use]
    pub fn archive_for(&self, target: &str) -> Option<(&str, &Artifact)> {
        self.artifacts
            .iter()
            .find(|(_, artifact)| {
                artifact.kind.as_deref() == Some("executable-zip")
                    && artifact
                        .target_triples
                        .iter()
                        .any(|triple| triple == target)
            })
            .map(|(name, artifact)| (name.as_str(), artifact))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINNED: &str = include_str!("../tests/fixtures/dist-manifest-1.14.0.json");

    #[test]
    fn a_real_manifest_says_everything_an_update_reads() {
        let manifest = Manifest::parse(PINNED).expect("parses");

        assert_eq!(manifest.version(), Some("1.14.0".parse().expect("valid")));
        assert_eq!(manifest.announcement_tag.as_deref(), Some("v1.14.0"));
        assert!(!manifest.announcement_is_prerelease);
        assert_eq!(
            manifest.released_on(),
            Some(jiff::civil::date(2026, 10, 14))
        );
        assert!(
            manifest
                .announcement_changelog
                .as_deref()
                .is_some_and(|notes| notes.contains("**Compatibility.**"))
        );

        let (name, archive) = manifest
            .archive_for("x86_64-pc-windows-msvc")
            .expect("a Windows archive");
        assert_eq!(name, "layover-cli-x86_64-pc-windows-msvc.zip");
        assert_eq!(archive.checksums.get("sha256").map(String::len), Some(64));
        assert!(
            archive
                .assets
                .iter()
                .any(|asset| asset.kind.as_deref() == Some("executable")
                    && asset.path.as_deref() == Some("layover.exe"))
        );
        for target in [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ] {
            assert!(manifest.archive_for(target).is_some(), "{target}");
        }
        assert!(
            manifest
                .archive_for("riscv64gc-unknown-linux-gnu")
                .is_none()
        );
    }

    #[test]
    fn fields_it_does_not_read_or_does_not_find_change_nothing() {
        let manifest =
            Manifest::parse(r#"{"announcement_tag":"v2.0.0","future":{"x":1}}"#).expect("parses");
        assert_eq!(manifest.version(), Some("2.0.0".parse().expect("valid")));
        assert_eq!(manifest.released_on(), None);
    }
}
