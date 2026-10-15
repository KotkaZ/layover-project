//! How the running binary was installed, and so what installing a release means for it.
//!
//! Decided from the binary's own path, not from a receipt: receipts and Cargo's records are hints,
//! and go stale — a machine updated by hand keeps a dist receipt for a version long gone. A binary
//! in a Cargo home, or anywhere else its user can write, is replaced with the verified prebuilt one.
//! One Homebrew or npm owns is left to them, and a person is told the command. One in a directory
//! its user cannot write is never elevated into: the person is told what to run.

use std::path::{Path, PathBuf};

/// How a binary was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// In a Cargo home's `bin`: put there by `cargo install` or by dist's installer.
    CargoHome {
        /// The directory.
        dir: PathBuf,
    },
    /// Anywhere else its user can write.
    Standalone {
        /// The directory.
        dir: PathBuf,
    },
    /// Under a Homebrew Cellar.
    Homebrew,
    /// In an npm package.
    Npm,
    /// In a directory its user cannot write.
    NotWritable {
        /// The directory.
        dir: PathBuf,
    },
}

impl Method {
    /// Whether Layover replaces the binary itself.
    #[must_use]
    pub fn swaps(&self) -> bool {
        matches!(self, Self::CargoHome { .. } | Self::Standalone { .. })
    }

    /// What a person runs instead, when Layover does not replace the binary itself.
    #[must_use]
    pub fn instead(&self, version: &str) -> Option<String> {
        match self {
            Self::CargoHome { .. } | Self::Standalone { .. } => None,
            Self::Homebrew => Some("brew upgrade KotkaZ/tap/layover".to_owned()),
            Self::Npm => Some(format!("npm install -g @layover/layover-cli@{version}")),
            Self::NotWritable { dir } => Some(format!(
                "install it the way it was installed into {}, which this user cannot write",
                dir.display()
            )),
        }
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CargoHome { dir } => write!(f, "Cargo home, {}", dir.display()),
            Self::Standalone { dir } => write!(f, "standalone, {}", dir.display()),
            Self::Homebrew => f.write_str("Homebrew"),
            Self::Npm => f.write_str("npm"),
            Self::NotWritable { dir } => write!(f, "{}, not writable", dir.display()),
        }
    }
}

/// How the binary at `binary` was installed, given the Cargo home `cargo_home` if there is one.
#[must_use]
pub fn detect(binary: &Path, cargo_home: Option<&Path>) -> Method {
    let real = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let text = real.to_string_lossy().replace('\\', "/");
    if text.contains("/Cellar/layover/") {
        return Method::Homebrew;
    }
    if text.contains("/node_modules/@layover/") {
        return Method::Npm;
    }
    let dir = binary
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    if !writable(&dir) {
        return Method::NotWritable { dir };
    }
    let in_cargo_home = cargo_home.is_some_and(|home| {
        let bin = home.join("bin");
        same_dir(&bin, &dir)
    });
    if in_cargo_home {
        Method::CargoHome { dir }
    } else {
        Method::Standalone { dir }
    }
}

/// The Cargo home: `CARGO_HOME`, or `.cargo` in the home directory.
#[must_use]
pub fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".cargo"))
        })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Whether this user can create a file in `dir`, found by trying.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".layover-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Brings a dist install receipt beside `binary` up to `version`, so whoever reads it next is not
/// told an old version is installed. Best effort: a receipt that is missing, for another
/// directory, or unreadable is left as it is.
pub fn update_receipt(binary: &Path, version: &str) {
    let Some(receipt) = receipt_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&receipt) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let prefix = value["install_prefix"].as_str().map(PathBuf::from);
    let dir = binary.parent();
    let ours = prefix.is_some_and(|prefix| {
        dir.is_some_and(|dir| same_dir(&prefix, dir) || same_dir(&prefix.join("bin"), dir))
    });
    if !ours {
        return;
    }
    value["version"] = serde_json::Value::String(version.to_owned());
    if let Ok(text) = serde_json::to_string(&value) {
        let _ = std::fs::write(&receipt, text);
    }
}

/// Where dist's installers keep the receipt.
fn receipt_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
    };
    Some(base.join("layover-cli").join("layover-cli-receipt.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("layover-method-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creates");
        dir
    }

    #[test]
    fn a_binary_in_a_cargo_home_is_replaced_whatever_its_receipt_says() {
        let home = temp("cargo");
        std::fs::create_dir_all(home.join("bin")).expect("creates");
        let binary = home.join("bin").join("layover.exe");
        std::fs::write(&binary, "x").expect("writes");

        let method = detect(&binary, Some(&home));

        assert!(matches!(method, Method::CargoHome { .. }), "{method:?}");
        assert!(method.swaps());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_binary_anywhere_else_writable_is_standalone() {
        let dir = temp("standalone");
        let binary = dir.join("layover");
        std::fs::write(&binary, "x").expect("writes");

        let method = detect(&binary, Some(Path::new("/no/cargo/home")));

        assert!(matches!(method, Method::Standalone { .. }), "{method:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_package_managers_binary_is_left_to_it() {
        let brew = Path::new("/opt/homebrew/Cellar/layover/1.14.0/bin/layover");
        assert_eq!(detect(brew, None), Method::Homebrew);
        assert_eq!(
            Method::Homebrew.instead("1.15.0").as_deref(),
            Some("brew upgrade KotkaZ/tap/layover")
        );
        let npm = Path::new("/usr/lib/node_modules/@layover/layover-cli/bin/layover");
        assert_eq!(detect(npm, None), Method::Npm);
        assert!(!Method::Npm.swaps());
    }

    #[test]
    fn a_directory_this_user_cannot_write_is_never_elevated_into() {
        let missing = Path::new("/layover-no-such-dir/bin/layover");
        assert!(matches!(detect(missing, None), Method::NotWritable { .. }));
    }
}
