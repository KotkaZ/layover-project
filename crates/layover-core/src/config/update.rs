//! `[update]`: whether this factory's Tower looks for a newer Layover, and what it does about one.
//!
//! Notifying is the default: the Tower asks once a day whether a newer release exists, and the
//! dashboard, `doctor` and `layover update check` say so. Nothing is downloaded and nothing
//! changes. See `docs/decisions.md` for why Layover updates itself at all.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use super::window::UpdateWindow;

/// How this factory's Tower treats new releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// Whether to look for new releases at all.
    #[serde(default)]
    pub mode: UpdateMode,
    /// Which releases to consider.
    #[serde(default)]
    pub channel: UpdateChannel,
    /// How often to look, in seconds. Written `check_every = "24h"`.
    #[serde(
        rename = "check_every",
        default = "default_check_every_sec",
        deserialize_with = "crate::duration::seconds"
    )]
    pub check_every_sec: u64,
    /// How long a Tower installing a release waits for what it is running to finish before
    /// cutting it off, in seconds. Written `max_drain = "4h"`.
    #[serde(
        rename = "max_drain",
        default = "default_max_drain_sec",
        deserialize_with = "crate::duration::seconds"
    )]
    pub max_drain_sec: u64,
    /// How long the binary an update replaced is kept for a rollback, in whole days. Written
    /// `keep_previous = "7d"`.
    #[serde(
        rename = "keep_previous",
        default = "default_keep_previous_days",
        deserialize_with = "crate::duration::days"
    )]
    pub keep_previous_days: u32,
    /// Under `mode = "auto"`, when an install may begin: `"Sat 02:00-06:00"`, local time. Without
    /// one, an install begins once the factory has been idle for `idle_for`.
    #[serde(default)]
    pub window: Option<UpdateWindow>,
    /// Under `mode = "auto"` with no `window`, how long nothing must have run or waited before an
    /// install begins, in seconds. Written `idle_for = "10m"`.
    #[serde(
        rename = "idle_for",
        default = "default_idle_for_sec",
        deserialize_with = "crate::duration::seconds"
    )]
    pub idle_for_sec: u64,
    /// How old a release must be before `mode = "auto"` installs it.
    #[serde(default)]
    pub min_age: MinAge,
    /// The largest move `mode = "auto"` makes. A major release, and one that changes the state
    /// layout, always waits for a person.
    #[serde(default)]
    pub allow: UpdateAllow,
    /// Whether `mode = "auto"` installs only what `gh` verified Layover's release workflow built.
    #[serde(default = "default_require_provenance")]
    pub require_provenance: bool,
}

/// Ten minutes: long enough that a factory between two runs of a busy schedule is not mistaken for
/// an idle one.
fn default_idle_for_sec() -> u64 {
    10 * 60
}

fn default_require_provenance() -> bool {
    true
}

/// How old a release must be before an unattended install takes it, per kind: the window in which
/// a bad release can be withdrawn before unattended factories have it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinAge {
    /// For a patch release, in seconds. Written `patch = "24h"`.
    #[serde(
        rename = "patch",
        default = "default_patch_age",
        deserialize_with = "crate::duration::seconds"
    )]
    pub patch_sec: u64,
    /// For a minor release, in seconds. Written `minor = "48h"`.
    #[serde(
        rename = "minor",
        default = "default_minor_age",
        deserialize_with = "crate::duration::seconds"
    )]
    pub minor_sec: u64,
}

fn default_patch_age() -> u64 {
    24 * 60 * 60
}

fn default_minor_age() -> u64 {
    48 * 60 * 60
}

impl Default for MinAge {
    fn default() -> Self {
        Self {
            patch_sec: default_patch_age(),
            minor_sec: default_minor_age(),
        }
    }
}

/// The largest move an unattended install makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAllow {
    /// Patch releases only.
    Patch,
    /// Patch and minor releases.
    #[default]
    Minor,
}

impl fmt::Display for UpdateAllow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
        })
    }
}

/// Four hours: as long as the longest run in the reference factory may take, so a drain that
/// begins with every slot busy ends without cutting anything off.
fn default_max_drain_sec() -> u64 {
    4 * 60 * 60
}

/// A week: long enough for a release that is healthy but wrong to be noticed and rolled back.
fn default_keep_previous_days() -> u32 {
    7
}

/// Once a day: a release a day old is still new, and asking more often only spends requests.
fn default_check_every_sec() -> u64 {
    24 * 60 * 60
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            mode: UpdateMode::default(),
            channel: UpdateChannel::default(),
            check_every_sec: default_check_every_sec(),
            max_drain_sec: default_max_drain_sec(),
            keep_previous_days: default_keep_previous_days(),
            window: None,
            idle_for_sec: default_idle_for_sec(),
            min_age: MinAge::default(),
            allow: UpdateAllow::default(),
            require_provenance: default_require_provenance(),
        }
    }
}

impl UpdateConfig {
    /// How often to look for a new release.
    #[must_use]
    pub fn check_every(&self) -> Duration {
        Duration::from_secs(self.check_every_sec)
    }

    /// How long an install waits for what is running to finish.
    #[must_use]
    pub fn max_drain(&self) -> Duration {
        Duration::from_secs(self.max_drain_sec)
    }

    /// How long the factory must be idle before an unattended install with no window begins.
    #[must_use]
    pub fn idle_for(&self) -> Duration {
        Duration::from_secs(self.idle_for_sec)
    }

    /// How long the replaced binary is kept.
    #[must_use]
    pub fn keep_previous(&self) -> Duration {
        Duration::from_secs(u64::from(self.keep_previous_days) * 24 * 60 * 60)
    }
}

/// Whether the Tower looks for new releases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Never look. A person can still run `layover update check`.
    Off,
    /// Look, and say what was found. Download nothing, change nothing.
    #[default]
    Notify,
    /// Look, and get a newer release ready: downloaded, verified, asked what it is and checked
    /// against this factory. Install nothing.
    Stage,
    /// Look, get it ready, and install it unattended — within the limits `allow`, `min_age`,
    /// `require_provenance` and `window` or `idle_for` set.
    Auto,
}

impl fmt::Display for UpdateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Notify => "notify",
            Self::Stage => "stage",
            Self::Auto => "auto",
        })
    }
}

/// Which releases are considered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    /// Releases only.
    #[default]
    Stable,
    /// Pre-releases too, such as `1.15.0-rc.1`, for trying one before stable factories do.
    Prerelease,
}

impl fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Prerelease => "prerelease",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn a_factory_that_says_nothing_is_told_about_stable_releases_daily() {
        let config = Config::from_toml("", "update.toml").expect("parses");

        assert_eq!(config.update.mode, UpdateMode::Notify);
        assert_eq!(config.update.channel, UpdateChannel::Stable);
        assert_eq!(config.update.check_every(), Duration::from_hours(24));
        assert_eq!(config.update.max_drain(), Duration::from_hours(4));
        assert_eq!(config.update.keep_previous(), Duration::from_hours(7 * 24));
    }

    #[test]
    fn every_key_can_be_set() {
        let config = Config::from_toml(
            "[update]\nmode = \"off\"\nchannel = \"prerelease\"\ncheck_every = \"6h\"\n\
             max_drain = \"90m\"\nkeep_previous = \"14d\"\n",
            "update.toml",
        )
        .expect("parses");

        assert_eq!(config.update.mode, UpdateMode::Off);
        assert_eq!(config.update.channel, UpdateChannel::Prerelease);
        assert_eq!(config.update.check_every(), Duration::from_hours(6));
        assert_eq!(config.update.max_drain(), Duration::from_mins(90));
        assert_eq!(config.update.keep_previous_days, 14);
    }

    #[test]
    fn auto_is_patch_and_minor_after_a_cool_off_when_idle_and_verified_by_default() {
        let config =
            Config::from_toml("[update]\nmode = \"auto\"\n", "update.toml").expect("parses");

        assert_eq!(config.update.mode, UpdateMode::Auto);
        assert_eq!(config.update.allow, UpdateAllow::Minor);
        assert_eq!(config.update.min_age, MinAge::default());
        assert_eq!(config.update.min_age.patch_sec, 24 * 3600);
        assert_eq!(config.update.min_age.minor_sec, 48 * 3600);
        assert_eq!(config.update.window, None);
        assert_eq!(config.update.idle_for(), Duration::from_mins(10));
        assert!(config.update.require_provenance);

        let config = Config::from_toml(
            "[update]\nmode = \"auto\"\nwindow = \"Sat 02:00-06:00\"\nallow = \"patch\"\n\
             min_age = { patch = \"12h\" }\nrequire_provenance = false\nidle_for = \"30m\"\n",
            "update.toml",
        )
        .expect("parses");
        assert_eq!(
            config
                .update
                .window
                .map(|window| window.to_string())
                .as_deref(),
            Some("Sat 02:00-06:00")
        );
        assert_eq!(config.update.allow, UpdateAllow::Patch);
        assert_eq!(config.update.min_age.patch_sec, 12 * 3600);
        assert_eq!(
            config.update.min_age.minor_sec,
            48 * 3600,
            "the other keeps its default"
        );
        assert!(!config.update.require_provenance);
        assert_eq!(config.update.idle_for(), Duration::from_mins(30));
    }

    #[test]
    fn an_unknown_key_or_mode_is_refused() {
        for text in [
            "[update]\nmode = \"sometimes\"\n",
            "[update]\nchanel = \"stable\"\n",
            "[update]\ncheck_every = \"daily\"\n",
            "[update]\nallow = \"major\"\n",
            "[update]\nwindow = \"weekends\"\n",
            "[update]\nmin_age = { major = \"1h\" }\n",
        ] {
            assert!(Config::from_toml(text, "update.toml").is_err(), "{text}");
        }
    }
}
