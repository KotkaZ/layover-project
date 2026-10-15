//! Whether an archive is what Layover's release workflow built, from its GitHub attestation.
//!
//! A checksum fetched from the same place as the file proves only that the two agree. Every
//! release carries a build-provenance attestation, signed through Sigstore with the identity of the
//! workflow that built it, and that is what proves who built it. `gh attestation verify` checks it
//! — the certificate, its transparency-log entry, and the identity — against pins that say exactly
//! which repository, workflow, tag and issuer it must name.
//!
//! Verifying inside the binary was tried first and rejected: the Sigstore crate refused a genuine
//! Layover bundle, and left the transparency-log proof unimplemented. See `docs/decisions.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::probe::{Environment, RunError, run};

/// The repository releases are built in.
pub const REPOSITORY: &str = "KotkaZ/layover-project";

/// The workflow that builds them.
pub const SIGNER_WORKFLOW: &str = "KotkaZ/layover-project/.github/workflows/release.yml";

/// Who issues the workflow's signing certificate.
pub const ISSUER: &str = "https://token.actions.githubusercontent.com";

/// What the attestation must say it is.
pub const PREDICATE: &str = "https://slsa.dev/provenance/v1";

/// What checking an archive's provenance found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Attestation {
    /// Built by Layover's release workflow, from the release's own tag.
    Verified {
        /// The workflow and ref that signed it, as the attestation says.
        signer: String,
    },
    /// It could not be checked here: no `gh`, `gh` not signed in, or GitHub out of reach. Such a
    /// release is never applied automatically, and is not run to ask what it is.
    Unavailable {
        /// Why not.
        reason: String,
    },
    /// It was checked, and is not what the release says. Nothing more is done with it.
    Refused {
        /// What `gh` said.
        reason: String,
    },
}

impl Attestation {
    /// Whether it was verified.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }
}

/// Something that can check an archive's provenance.
pub trait Verifier {
    /// Checks `archive`, which the release tagged `tag` lists.
    fn verify(&self, archive: &Path, tag: &str) -> Attestation;
}

/// The GitHub CLI.
#[derive(Debug, Clone)]
pub struct Gh {
    program: PathBuf,
}

impl Default for Gh {
    fn default() -> Self {
        Self {
            program: PathBuf::from("gh"),
        }
    }
}

impl Gh {
    /// The `gh` at `program`.
    #[must_use]
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl Verifier for Gh {
    fn verify(&self, archive: &Path, tag: &str) -> Attestation {
        let archive_text = archive.display().to_string();
        let source_ref = format!("refs/tags/{tag}");
        let args = [
            "attestation",
            "verify",
            archive_text.as_str(),
            "--repo",
            REPOSITORY,
            "--signer-workflow",
            SIGNER_WORKFLOW,
            "--source-ref",
            source_ref.as_str(),
            "--cert-oidc-issuer",
            ISSUER,
            "--predicate-type",
            PREDICATE,
            "--format",
            "json",
        ];
        let dir = archive.parent().unwrap_or_else(|| Path::new("."));
        match run(
            &self.program,
            &args,
            dir,
            Environment::Inherited,
            Duration::from_mins(2),
        ) {
            Err(RunError::NotFound(_)) => Attestation::Unavailable {
                reason: "the GitHub CLI, gh, is not installed: https://cli.github.com".to_owned(),
            },
            Err(error) => Attestation::Unavailable {
                reason: error.to_string(),
            },
            Ok(ran) => judge(ran.code, &ran.stdout, &ran.stderr, tag),
        }
    }
}

/// What `gh attestation verify` ending with `code` and printing `stdout` and `stderr` means.
fn judge(code: Option<i32>, stdout: &str, stderr: &str, tag: &str) -> Attestation {
    let said = first_line(stderr).unwrap_or_else(|| "gh said nothing".to_owned());
    match code {
        Some(0) => Attestation::Verified {
            signer: signer(stdout).unwrap_or_else(|| format!("{SIGNER_WORKFLOW}@refs/tags/{tag}")),
        },
        // gh's own code for "sign in first".
        Some(4) => Attestation::Unavailable {
            reason: format!("gh is not signed in; run `gh auth login`. ({said})"),
        },
        _ if could_not_ask(stderr) => Attestation::Unavailable { reason: said },
        _ => Attestation::Refused { reason: said },
    }
}

/// Whether `gh` failed for want of a network or a login rather than because of the archive.
fn could_not_ask(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    [
        "gh auth login",
        "authentication",
        "http 401",
        "http 403",
        "rate limit",
        "dial tcp",
        "no such host",
        "connection",
        "timeout",
        "tls",
        "unknown command",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// The signing workflow and ref, from `--format json`'s first result.
fn signer(stdout: &str) -> Option<String> {
    let results: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let certificate = &results.get(0)?["verificationResult"]["signature"]["certificate"];
    certificate["buildSignerURI"]
        .as_str()
        .or_else(|| certificate["subjectAlternativeName"].as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_exit_is_verified_and_names_its_signer() {
        let stdout = r#"[{"verificationResult":{"signature":{"certificate":{
            "buildSignerURI":"https://github.com/KotkaZ/layover-project/.github/workflows/release.yml@refs/tags/v1.15.0"}}}}]"#;
        assert_eq!(
            judge(Some(0), stdout, "", "v1.15.0"),
            Attestation::Verified {
                signer: "https://github.com/KotkaZ/layover-project/.github/workflows/release.yml@refs/tags/v1.15.0"
                    .to_owned()
            }
        );
        assert!(judge(Some(0), "not json", "", "v1.15.0").is_verified());
    }

    #[test]
    fn an_archive_that_is_not_what_was_built_is_refused() {
        for stderr in [
            "Error: failed to fetch attestations: HTTP 404: Not Found",
            "Error: verifying with issuer \"sigstore.dev\"\nexpected SourceRepositoryRef to be refs/tags/v1.15.0",
        ] {
            assert!(
                matches!(
                    judge(Some(1), "", stderr, "v1.15.0"),
                    Attestation::Refused { .. }
                ),
                "{stderr}"
            );
        }
    }

    #[test]
    fn not_being_able_to_ask_is_not_a_refusal() {
        assert!(matches!(
            judge(
                Some(4),
                "",
                "To get started with GitHub CLI, please run:  gh auth login",
                "v1"
            ),
            Attestation::Unavailable { .. }
        ));
        assert!(matches!(
            judge(
                Some(1),
                "",
                "Post \"https://api.github.com\": dial tcp: no such host",
                "v1"
            ),
            Attestation::Unavailable { .. }
        ));
    }

    #[test]
    fn no_gh_is_unavailable() {
        let attestation = Gh::at("layover-no-such-gh")
            .verify(&std::env::temp_dir().join("nothing.zip"), "v1.15.0");
        assert!(
            matches!(&attestation, Attestation::Unavailable { reason } if reason.contains("not installed")),
            "{attestation:?}"
        );
    }
}
