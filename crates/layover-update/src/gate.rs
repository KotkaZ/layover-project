//! Whether a staged release would run this factory: the gate.
//!
//! The staged binary validates the factory's `layover.toml`, as `layover validate` would. Errors
//! refuse it, for any update. Warnings are compared with the running version's own for the same
//! file: one it did not have refuses an *automatic* update and is shown before a manual one. A
//! warning the factory has always had — an `overlap = "allow"` someone chose — holds nothing back.
//!
//! The gate is per factory, because two factories on one machine can disagree about a release, and
//! is kept in that factory's `.layover/update/gate.json`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::probe::{Environment, run};
use crate::stage::Staged;
use crate::version::Version;

/// What a staged release made of a factory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    /// The release.
    pub version: Version,
    /// The factory's definition.
    pub factory: PathBuf,
    /// When it was asked.
    pub at: Timestamp,
    /// Why the gate could not be run, when it could not.
    #[serde(default)]
    pub not_run: Option<String>,
    /// What the staged release refuses in the factory.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Warnings the staged release gives that the running one does not.
    #[serde(default)]
    pub new_warnings: Vec<String>,
}

impl Gate {
    /// Whether a person may install it: it ran, and refused nothing.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.not_run.is_none() && self.errors.is_empty()
    }

    /// Whether it may be installed with nobody watching: it passes, and says nothing new.
    #[must_use]
    pub fn passes_unattended(&self) -> bool {
        self.passes() && self.new_warnings.is_empty()
    }
}

/// Asks the release in `staged` to validate `factory`, measured against `running_warnings` — the
/// running version's own warnings for it, worded as `layover validate` prints them.
#[must_use]
pub fn gate(staged: &Staged, factory: &Path, running_warnings: &[String]) -> Gate {
    let mut gate = Gate {
        version: staged.version.clone(),
        factory: factory.to_path_buf(),
        at: Timestamp::now(),
        not_run: None,
        errors: Vec::new(),
        new_warnings: Vec::new(),
    };
    let (Some(binary), true) = (&staged.binary, staged.is_ready()) else {
        gate.not_run = Some(staged.refused.clone().unwrap_or_else(|| {
            "it is not ready to run: its provenance could not be checked".to_owned()
        }));
        return gate;
    };
    let factory_text = factory.display().to_string();
    let dir = binary.parent().unwrap_or_else(|| Path::new("."));
    let ran = match run(
        binary,
        &["validate", "--config", &factory_text],
        dir,
        Environment::Scrubbed,
        Duration::from_mins(1),
    ) {
        Ok(ran) => ran,
        Err(error) => {
            gate.not_run = Some(error.to_string());
            return gate;
        }
    };
    let said = Said::read(&ran.stdout, &ran.stderr);
    if ran.code == Some(0) {
        gate.new_warnings = said
            .warnings
            .into_iter()
            .filter(|warning| !running_warnings.contains(warning))
            .collect();
    } else if said.errors.is_empty() {
        gate.errors = vec![
            ran.stderr
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("it refused the factory without saying why")
                .trim()
                .to_owned(),
        ];
    } else {
        gate.errors = said.errors;
    }
    gate
}

/// The findings `layover validate` printed, without their labels.
struct Said {
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Said {
    /// Reads what `validate` printed. A failing run prints its findings to stderr behind the
    /// binary's own `error: `, so the first line may carry two labels.
    fn read(stdout: &str, stderr: &str) -> Self {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        for (index, line) in stderr.lines().enumerate().chain(stdout.lines().enumerate()) {
            let mut line = line.trim();
            if index == 0
                && let Some(rest) = line.strip_prefix("error: ")
                && (rest.starts_with("error: ") || rest.starts_with("warning: "))
            {
                line = rest;
            }
            if let Some(found) = line.strip_prefix("error: ") {
                errors.push(found.to_owned());
            } else if let Some(found) = line.strip_prefix("warning: ") {
                warnings.push(found.to_owned());
            }
        }
        Self { errors, warnings }
    }
}

/// Where a factory's gate is kept: `.layover/update/gate.json` beside its definition.
#[must_use]
pub fn path_for(factory: &Path) -> PathBuf {
    factory
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".layover")
        .join("update")
        .join("gate.json")
}

/// Records `gate` for its factory.
///
/// # Errors
///
/// Returns the I/O error when it cannot be written.
pub fn record(gate: &Gate) -> std::io::Result<()> {
    let path = path_for(&gate.factory);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(gate).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// The gate last recorded for `factory`.
#[must_use]
pub fn recorded(factory: &Path) -> Option<Gate> {
    let text = std::fs::read_to_string(path_for(factory)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_are_read_as_validate_prints_them() {
        let said = Said::read(
            "warning: pipeline `sweep` may overlap\n1 agent(s), 1 pipeline(s), 0 route(s) — 1 warning(s), no errors\n",
            "",
        );
        assert_eq!(said.warnings, ["pipeline `sweep` may overlap"]);
        assert!(said.errors.is_empty());

        let said = Said::read(
            "",
            "error: error: unknown agent `ghost`\nwarning: agent `a` has no description\n\n1 error(s), 1 warning(s) — this factory will not start\n",
        );
        assert_eq!(said.errors, ["unknown agent `ghost`"]);
        assert_eq!(said.warnings, ["agent `a` has no description"]);
    }

    #[test]
    fn a_warning_the_factory_always_had_holds_nothing_back() {
        let mut gate = Gate {
            version: "1.15.0".parse().expect("valid"),
            factory: PathBuf::from("layover.toml"),
            at: Timestamp::now(),
            not_run: None,
            errors: Vec::new(),
            new_warnings: Vec::new(),
        };
        assert!(gate.passes_unattended());

        gate.new_warnings.push("something new".to_owned());
        assert!(gate.passes() && !gate.passes_unattended());

        gate.errors.push("refused".to_owned());
        assert!(!gate.passes());
    }
}
