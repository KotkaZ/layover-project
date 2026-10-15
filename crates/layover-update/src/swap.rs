//! Replacing the running binary with a staged one, and putting it back.
//!
//! Everything happens in the binary's own directory, so every rename stays on one volume and is
//! atomic. The binary being replaced is never deleted or overwritten — Windows forbids both while
//! it runs — only renamed aside, where it becomes the copy a rollback restores:
//!
//! 1. the staged binary is copied beside the current one as `layover.exe.new`, and its SHA-256
//!    compared with the staged file's;
//! 2. the current binary is renamed (Windows) or hard-linked (elsewhere) to `layover-<from>.old`;
//! 3. `layover.exe.new` is renamed over the current name.
//!
//! If step 3 fails, step 2 is undone. A running Tower keeps the image it started from either way.
//! Each rename is retried for up to thirty seconds, because antivirus scanners briefly hold a new
//! executable open. All of it runs under the install lock, a file lock in the machine's update
//! directory, so two Towers on one machine never swap at once.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::Settings;
use crate::download::digest_of;
use crate::version::Version;

/// How long a rename is retried while something holds the file.
const PATIENCE: Duration = Duration::from_secs(30);

/// A binary replaced, and where the one it replaced was kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swapped {
    /// The binary's path, which now holds the new version.
    pub binary: PathBuf,
    /// The version it replaced, kept beside it.
    pub previous: PathBuf,
}

/// The install lock: held while a binary is swapped, released however the process ends.
#[derive(Debug)]
pub struct InstallLock(File);

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Takes the machine's install lock.
///
/// # Errors
///
/// Says another process holds it, or that it could not be taken.
pub fn install_lock(settings: &Settings) -> Result<InstallLock, String> {
    std::fs::create_dir_all(&settings.dir).map_err(|error| error.to_string())?;
    let path = settings.dir.join("install.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(InstallLock(file)),
        Err(TryLockError::WouldBlock) => Err(
            "another Layover on this machine is installing a release; try again once it has"
                .to_owned(),
        ),
        Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
}

/// Where the version `from` is kept beside `binary`: `layover-1.14.0.exe.old`.
#[must_use]
pub fn previous_path(binary: &Path, from: &Version) -> PathBuf {
    let stem = binary
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("layover");
    let name = match binary.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => format!("{stem}-{from}.{extension}.old"),
        None => format!("{stem}-{from}.old"),
    };
    binary.with_file_name(name)
}

/// Replaces `binary`, which is version `from`, with `staged`.
///
/// # Errors
///
/// Says which step failed. Whatever failed, `binary` is the version it was.
pub fn swap(binary: &Path, staged: &Path, from: &Version) -> Result<Swapped, String> {
    let fresh = with_suffix(binary, ".new");
    let previous = previous_path(binary, from);

    // 1. Beside it, and the same bytes as what was verified.
    let _ = std::fs::remove_file(&fresh);
    std::fs::copy(staged, &fresh)
        .map_err(|error| format!("could not copy the new binary beside the old: {error}"))?;
    let expected = digest_of(staged).map_err(|error| error.to_string())?;
    let copied = digest_of(&fresh).map_err(|error| error.to_string())?;
    if copied != expected {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!(
            "the copy beside the old binary reads {copied}, not {expected}"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("could not make the new binary executable: {error}"))?;
    }

    // 2. The old one aside. A copy kept by an earlier update is replaced; one still held open by
    //    something is left, and this one kept under another name.
    let previous = if previous.exists() && std::fs::remove_file(&previous).is_err() {
        with_suffix(&previous, &format!(".{}", std::process::id()))
    } else {
        previous
    };
    if let Err(error) = keep_aside(binary, &previous) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("could not set the old binary aside: {error}"));
    }

    // 3. The new one in. Undone if it fails, so the old binary is never missing.
    if let Err(error) = retrying(|| std::fs::rename(&fresh, binary)) {
        let _ = put_back(binary, &previous);
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("could not put the new binary in place: {error}"));
    }
    Ok(Swapped {
        binary: binary.to_path_buf(),
        previous,
    })
}

/// Puts the version kept in `swapped.previous` back in place.
///
/// # Errors
///
/// Says why it could not be.
pub fn restore(swapped: &Swapped) -> Result<(), String> {
    let failed = with_suffix(&swapped.binary, ".failed");
    let _ = std::fs::remove_file(&failed);
    // The failed binary may be running — the Tower that failed to become healthy — so it is
    // renamed aside rather than deleted.
    retrying(|| std::fs::rename(&swapped.binary, &failed))
        .map_err(|error| format!("could not set the new binary aside: {error}"))?;
    if let Err(error) = retrying(|| copy_back(&swapped.previous, &swapped.binary)) {
        let _ = std::fs::rename(&failed, &swapped.binary);
        return Err(format!("could not put the old binary back: {error}"));
    }
    let _ = std::fs::remove_file(&failed);
    Ok(())
}

/// Removes the kept previous binary, once the new one has proved itself and been kept long enough.
pub fn forget_previous(previous: &Path) {
    let _ = std::fs::remove_file(previous);
}

#[cfg(windows)]
fn keep_aside(binary: &Path, previous: &Path) -> std::io::Result<()> {
    // A running executable may be renamed but not deleted or overwritten.
    retrying(|| std::fs::rename(binary, previous))
}

#[cfg(not(windows))]
fn keep_aside(binary: &Path, previous: &Path) -> std::io::Result<()> {
    // A second name for the same file, so the current name is never missing before step 3.
    std::fs::hard_link(binary, previous).or_else(|_| std::fs::copy(binary, previous).map(|_| ()))
}

#[cfg(windows)]
fn put_back(binary: &Path, previous: &Path) -> std::io::Result<()> {
    retrying(|| std::fs::rename(previous, binary))
}

#[cfg(not(windows))]
fn put_back(_binary: &Path, previous: &Path) -> std::io::Result<()> {
    // The current name still holds the old binary; only the extra name goes.
    std::fs::remove_file(previous)
}

/// Copies the kept binary back rather than renaming it, so the copy kept for a rollback stays
/// until it is forgotten on purpose.
fn copy_back(previous: &Path, binary: &Path) -> std::io::Result<()> {
    std::fs::copy(previous, binary).map(|_| ())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Retries `step` while it fails, for up to [`PATIENCE`].
fn retrying(mut step: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let began = Instant::now();
    loop {
        match step() {
            Ok(()) => return Ok(()),
            Err(error) if began.elapsed() >= PATIENCE => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        text.parse().expect("valid")
    }

    #[test]
    fn the_kept_copy_is_named_for_its_version() {
        assert_eq!(
            previous_path(Path::new("bin/layover.exe"), &v("1.14.0")),
            PathBuf::from("bin/layover-1.14.0.exe.old")
        );
        assert_eq!(
            previous_path(Path::new("bin/layover"), &v("1.14.0")),
            PathBuf::from("bin/layover-1.14.0.old")
        );
    }
}
