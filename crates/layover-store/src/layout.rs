//! The version stamped on a factory's state directory.
//!
//! # Why a version key exists at all
//!
//! `.layover/` accumulates things a factory cannot recreate: what ran, what it cost, what agents
//! learned, what they were blocked on, and work that has been queued but not started. Two releases
//! silently disagreeing about the shape of any of that is a data-loss bug waiting for its moment —
//! and the moment it picks is an upgrade, which is exactly when nobody is watching the details.
//!
//! # Why a newer layout is refused rather than read hopefully
//!
//! An older binary opening a newer directory cannot know what it does not understand. Reading it
//! anyway means writing it back without whatever was added, which turns "I downgraded for an
//! afternoon" into permanent loss. Refusing is recoverable: the operator upgrades again, or points
//! at a different directory.
//!
//! An older layout is the opposite case — this binary knows exactly what changed — so it is
//! migrated forward once, and said out loud, because a silent migration is indistinguishable from
//! a silent corruption until much later.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The layout this build writes and understands.
///
/// Bumped when the *shape* of anything under `.layover/` changes in a way an older build would
/// misread. Adding a field that older builds ignore does not need a bump; moving, renaming or
/// re-meaning one does.
pub const CURRENT: u32 = 1;

/// The file the version is kept in.
const MARKER: &str = "version.json";

/// What is written in the state directory to say which layout it is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Marker {
    /// The layout version.
    pub layout: u32,
    /// The release that last wrote it, for a human reading the file.
    ///
    /// Never compared against. Layout compatibility is the layout number's job, and two builds of
    /// the same layout must be interchangeable or the number means nothing.
    pub written_by: String,
}

/// Why a state directory could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incompatible {
    /// The directory was written by a newer release.
    Newer {
        /// What is on disk.
        found: u32,
        /// What this build understands.
        supported: u32,
        /// The release that wrote it, as recorded.
        written_by: String,
    },
    /// The marker exists and could not be read.
    Unreadable {
        /// What went wrong.
        detail: String,
    },
}

impl std::fmt::Display for Incompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Newer {
                found,
                supported,
                written_by,
            } => write!(
                f,
                "this state directory is layout {found}, written by Layover {written_by}, and \
                 this build understands layout {supported}. Upgrade, or point at a different \
                 directory — reading it anyway would drop whatever the newer release added."
            ),
            Self::Unreadable { detail } => write!(
                f,
                "the state directory's version marker could not be read: {detail}. It is \
                 `.layover/{MARKER}`; delete it only if you are certain the directory is empty."
            ),
        }
    }
}

impl std::error::Error for Incompatible {}

/// What opening a state directory did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// It was already this layout.
    Current,
    /// It had no marker and has been stamped.
    ///
    /// Either a fresh directory or one from before versioning existed. Both are treated as this
    /// layout, which is correct for the first versioned release and is the reason to introduce
    /// versioning before the shape ever changes rather than after.
    Stamped,
    /// It was older and has been brought forward.
    Migrated {
        /// What it was.
        from: u32,
    },
}

impl std::fmt::Display for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => f.write_str("state directory is current"),
            Self::Stamped => write!(f, "stamped the state directory as layout {CURRENT}"),
            Self::Migrated { from } => {
                write!(
                    f,
                    "migrated the state directory from layout {from} to {CURRENT}"
                )
            }
        }
    }
}

/// Checks a state directory's layout, stamping or migrating it as needed.
///
/// Call this before anything reads or writes `.layover/`.
///
/// # Errors
///
/// Returns [`Incompatible`] when the directory was written by a newer release, or when its marker
/// exists and cannot be read. A directory that cannot be *written* is not an error here: a
/// read-only `.layover/` still answers questions, and refusing to start over a marker nobody needs
/// would make the dashboard useless in exactly the situation it is most wanted.
pub fn open(root: &Path) -> Result<Opened, Incompatible> {
    let path = marker_path(root);

    let found = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Marker>(&text) {
            Ok(marker) => Some(marker),
            Err(error) => {
                return Err(Incompatible::Unreadable {
                    detail: error.to_string(),
                });
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Incompatible::Unreadable {
                detail: error.to_string(),
            });
        }
    };

    let Some(marker) = found else {
        stamp(root);
        return Ok(Opened::Stamped);
    };

    if marker.layout > CURRENT {
        return Err(Incompatible::Newer {
            found: marker.layout,
            supported: CURRENT,
            written_by: marker.written_by,
        });
    }

    if marker.layout < CURRENT {
        // No migration steps exist yet, because no shape has changed yet. When one does, it runs
        // here — once, before anything reads the directory — and the stamp is what stops it
        // running twice.
        stamp(root);
        return Ok(Opened::Migrated {
            from: marker.layout,
        });
    }

    Ok(Opened::Current)
}

/// Writes the marker, best-effort.
///
/// Deliberately ignores failure. A `.layover/` that cannot be written is a real problem, but it is
/// one the next actual write will report against the thing somebody was trying to do — which is a
/// better error than "could not write a version file".
fn stamp(root: &Path) {
    let marker = Marker {
        layout: CURRENT,
        written_by: env!("CARGO_PKG_VERSION").to_owned(),
    };

    if std::fs::create_dir_all(root).is_err() {
        return;
    }

    if let Ok(text) = serde_json::to_string_pretty(&marker) {
        let _ = std::fs::write(marker_path(root), text);
    }
}

/// Where the marker lives.
fn marker_path(root: &Path) -> PathBuf {
    root.join(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-layout-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temporary directory");
            Self(path)
        }

        fn write_marker(&self, text: &str) {
            std::fs::write(self.0.join(MARKER), text).expect("writes");
        }

        fn marker(&self) -> String {
            std::fs::read_to_string(self.0.join(MARKER)).expect("a marker")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_fresh_directory_is_stamped_with_this_layout() {
        let temp = Temp::new("fresh");

        let opened = open(&temp.0).expect("a fresh directory is fine");

        assert_eq!(opened, Opened::Stamped);
        assert!(temp.marker().contains("\"layout\": 1"), "{}", temp.marker());
    }

    #[test]
    fn a_directory_from_before_versioning_is_treated_as_this_layout() {
        // There is no earlier shape to migrate from, so an unmarked directory is this one. That
        // is only true because versioning arrived before the shape ever changed, which is the
        // reason to do it now rather than after.
        let temp = Temp::new("unmarked");
        std::fs::write(temp.0.join("runs-2026-09-18.jsonl"), "{}\n").expect("writes history");

        let opened = open(&temp.0).expect("an unmarked directory is fine");

        assert_eq!(opened, Opened::Stamped);
        assert!(
            temp.0.join("runs-2026-09-18.jsonl").exists(),
            "stamping must not disturb what is already there"
        );
    }

    #[test]
    fn opening_a_current_directory_changes_nothing() {
        let temp = Temp::new("current");
        open(&temp.0).expect("stamps");
        let before = temp.marker();

        let opened = open(&temp.0).expect("still fine");

        assert_eq!(opened, Opened::Current);
        assert_eq!(temp.marker(), before, "a no-op must not rewrite the file");
    }

    #[test]
    fn a_newer_layout_is_refused_rather_than_read_hopefully() {
        // An older binary cannot know what it does not understand. Reading it anyway means writing
        // it back without whatever was added, which turns a downgrade into permanent loss.
        let temp = Temp::new("newer");
        temp.write_marker(r#"{"layout": 99, "written_by": "9.9.9"}"#);

        let error = open(&temp.0).expect_err("a newer layout must be refused");

        assert!(
            matches!(error, Incompatible::Newer { found: 99, .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("Upgrade"), "{error}");
        assert!(
            temp.marker().contains("99"),
            "a refused directory must be left exactly as it was"
        );
    }

    #[test]
    fn a_marker_that_cannot_be_parsed_stops_everything_rather_than_being_overwritten() {
        // Assuming a corrupt marker means "fresh" would stamp it as current and let a genuinely
        // newer directory be written back by an older build.
        let temp = Temp::new("corrupt");
        temp.write_marker("{ this is not json");

        let error = open(&temp.0).expect_err("a corrupt marker must be refused");

        assert!(
            matches!(error, Incompatible::Unreadable { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("version.json"), "{error}");
    }

    #[test]
    fn an_older_layout_is_brought_forward_and_says_so() {
        // A silent migration is indistinguishable from a silent corruption until much later.
        let temp = Temp::new("older");
        temp.write_marker(r#"{"layout": 0, "written_by": "0.0.1"}"#);

        let opened = open(&temp.0).expect("an older layout is migrated");

        assert_eq!(opened, Opened::Migrated { from: 0 });
        assert!(opened.to_string().contains("migrated"), "{opened}");
        assert!(temp.marker().contains("\"layout\": 1"), "{}", temp.marker());
    }

    #[test]
    fn the_marker_records_which_release_wrote_it() {
        // For a person reading the file. It is never compared against: two builds of one layout
        // must be interchangeable, or the layout number means nothing.
        let temp = Temp::new("written-by");
        open(&temp.0).expect("stamps");

        assert!(
            temp.marker().contains(env!("CARGO_PKG_VERSION")),
            "{}",
            temp.marker()
        );
    }
}
