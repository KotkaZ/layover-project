//! Release versions, and how far apart two of them are.
//!
//! Semantic versions as Layover tags them: `1.15.0`, or `1.15.0-rc.1` for a pre-release. Written
//! here rather than taken from a crate because comparing them is all an update needs, and a
//! version that does not parse is a release this updater does not consider.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A release version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    /// Incompatible changes.
    pub major: u64,
    /// Additions.
    pub minor: u64,
    /// Fixes.
    pub patch: u64,
    /// Dot-separated pre-release identifiers, empty for a release.
    pub pre: Vec<String>,
}

/// How far an update moves: the first part of the version that changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Only the patch number, or only the pre-release, changes.
    Patch,
    /// The minor number changes.
    Minor,
    /// The major number changes.
    Major,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        })
    }
}

impl Version {
    /// The version of this binary.
    ///
    /// # Panics
    ///
    /// Never in a build of this workspace, whose version is a valid one.
    #[must_use]
    pub fn running() -> Self {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("the workspace version is a semantic version")
    }

    /// Whether this is a pre-release, such as `1.15.0-rc.1`.
    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// How far moving from `self` to `to` goes.
    #[must_use]
    pub fn kind_of_move_to(&self, to: &Self) -> Kind {
        if self.major != to.major {
            Kind::Major
        } else if self.minor != to.minor {
            Kind::Minor
        } else {
            Kind::Patch
        }
    }
}

/// Why a version could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a version such as 1.15.0 or 1.15.0-rc.1")]
pub struct NotAVersion(String);

impl FromStr for Version {
    type Err = NotAVersion;

    /// Reads `1.15.0`, `1.15.0-rc.1`, or either with a leading `v` as a tag is written. Build
    /// metadata after `+` is not part of a Layover version and is refused.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || NotAVersion(text.to_owned());
        let bare = text.strip_prefix('v').unwrap_or(text);
        let (core, pre) = match bare.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (bare, None),
        };
        let mut numbers = core.split('.').map(|part| {
            // A leading zero is not a version number but something else written like one.
            if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
                return None;
            }
            if !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        });
        let (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) = (
            numbers.next(),
            numbers.next(),
            numbers.next(),
            numbers.next(),
        ) else {
            return Err(bad());
        };
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => {
                let parts: Vec<String> = pre.split('.').map(str::to_owned).collect();
                let valid = parts.iter().all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                });
                if !valid {
                    return Err(bad());
                }
                parts
            }
        };
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

impl Ord for Version {
    /// Semantic-version precedence: numbers first, then a pre-release before its release, then
    /// pre-release identifiers one by one — numeric ones by value and below any word.
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => compare_pre(&self.pre, &other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_pre(left: &[String], right: &[String]) -> Ordering {
    for (a, b) in left.iter().zip(right) {
        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        text.parse().expect("a version")
    }

    #[test]
    fn versions_read_as_layover_tags_them() {
        assert_eq!(v("v1.14.0"), v("1.14.0"));
        assert_eq!(v("1.15.0-rc.1").pre, vec!["rc", "1"]);
        assert_eq!(v("1.15.0-rc.1").to_string(), "1.15.0-rc.1");
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.x",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3+b",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn precedence_follows_semantic_versioning() {
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.2.0",
            "1.10.0",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn a_move_is_as_big_as_the_first_number_that_changes() {
        assert_eq!(v("1.14.0").kind_of_move_to(&v("1.14.1")), Kind::Patch);
        assert_eq!(v("1.14.3").kind_of_move_to(&v("1.15.0")), Kind::Minor);
        assert_eq!(v("1.14.0").kind_of_move_to(&v("2.0.0")), Kind::Major);
        assert_eq!(v("1.15.0-rc.1").kind_of_move_to(&v("1.15.0")), Kind::Patch);
    }

    #[test]
    fn the_running_version_reads() {
        assert!(!Version::running().to_string().is_empty());
    }
}
