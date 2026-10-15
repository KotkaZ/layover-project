//! Taking the binary out of a release archive, and nothing else.
//!
//! A release archive is a zip on Windows and an xz-compressed tar elsewhere, holding the binary
//! beside its README, licence and changelog — at the top of a zip, in one folder of a tar. Only the
//! binary is written, to a name this code chooses, so no entry can place a file anywhere else. An
//! archive with any entry that tries — an absolute path, a `..`, a link — is refused outright
//! anyway: it is not what the release workflow builds, and something that is not is not trusted.

use std::io::Read;
use std::path::{Component, Path};

/// The most a binary may unpack to. Layover's is about eleven megabytes.
pub const MAX_UNPACKED: u64 = 256 * 1024 * 1024;

/// Writes the executable `name` from `archive` to `to`.
///
/// # Errors
///
/// Returns why the archive was refused: unreadable, unsafe, without the binary, or too large.
pub fn executable(archive: &Path, name: &str, to: &Path) -> Result<(), String> {
    let file_name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if archive
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        from_zip(archive, name, to)
    } else if file_name.ends_with(".tar.xz") {
        from_tar_xz(archive, name, to)
    } else {
        Err(format!("{file_name} is neither a .zip nor a .tar.xz"))
    }
}

/// Whether `path`, as an archive names it, stays inside wherever it is unpacked.
fn stays_inside(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        && !path.split(['/', '\\']).any(|part| part == "..")
}

/// Whether the entry at `path` is the binary: at the top, or in one folder.
fn is_the_binary(path: &str, name: &str) -> bool {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    matches!(parts.as_slice(), [only] if *only == name)
        || matches!(parts.as_slice(), [_, last] if *last == name)
}

fn from_zip(archive: &Path, name: &str, to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| format!("not a zip: {error}"))?;
    let mut found = None;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let path = entry
            .name()
            .map_err(|error| format!("an entry has no readable name: {error}"))?
            .into_owned();
        if !stays_inside(&path) {
            return Err(format!("it holds `{path}`, which would land outside it"));
        }
        if entry.is_symlink() {
            return Err(format!("it holds `{path}`, a link"));
        }
        if entry.is_file() && is_the_binary(&path, name) {
            found = Some(index);
        }
    }
    let index = found.ok_or_else(|| format!("it holds no `{name}`"))?;
    let entry = zip.by_index(index).map_err(|error| error.to_string())?;
    write_capped(entry, to)
}

fn from_tar_xz(archive: &Path, name: &str, to: &Path) -> Result<(), String> {
    let mut packed =
        std::io::BufReader::new(std::fs::File::open(archive).map_err(|error| error.to_string())?);
    let mut tar_bytes = Vec::new();
    lzma_rs::xz_decompress(
        &mut packed,
        &mut CappedWriter {
            inner: &mut tar_bytes,
            left: MAX_UNPACKED,
        },
    )
    .map_err(|error| format!("not an xz archive, or larger than allowed: {error}"))?;

    let mut tar = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let mut written = false;
    for entry in tar.entries().map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| format!("not a tar: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        if !stays_inside(&path) {
            return Err(format!("it holds `{path}`, which would land outside it"));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(format!("it holds `{path}`, a link"));
        }
        if kind.is_file() && is_the_binary(&path, name) && !written {
            write_capped(entry, to)?;
            written = true;
        }
    }
    if written {
        Ok(())
    } else {
        Err(format!("it holds no `{name}`"))
    }
}

/// Writes `reader` to `to`, refusing more than [`MAX_UNPACKED`], and makes it executable.
fn write_capped(reader: impl Read, to: &Path) -> Result<(), String> {
    let mut limited = reader.take(MAX_UNPACKED + 1);
    let mut file = std::fs::File::create(to).map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut limited, &mut file).map_err(|error| error.to_string())?;
    if copied > MAX_UNPACKED {
        drop(file);
        let _ = std::fs::remove_file(to);
        return Err(format!(
            "its binary unpacks to more than {MAX_UNPACKED} bytes"
        ));
    }
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// A writer that refuses to grow past a limit, so an archive cannot decompress without bound.
struct CappedWriter<'a> {
    inner: &'a mut Vec<u8>,
    left: u64,
}

impl std::io::Write for CappedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if len > self.left {
            return Err(std::io::Error::other("larger than allowed"));
        }
        self.left -= len;
        self.inner.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_that_stay_inside_are_accepted() {
        for good in [
            "layover.exe",
            "layover-cli-x86_64-unknown-linux-gnu/layover",
            "./README.md",
        ] {
            assert!(stays_inside(good), "{good}");
        }
        for bad in [
            "../layover",
            "dir/../../layover",
            "/usr/bin/layover",
            "\\Windows\\layover.exe",
            "C:\\layover.exe",
            "C:layover.exe",
            "",
        ] {
            assert!(!stays_inside(bad), "{bad}");
        }
    }

    #[test]
    fn the_binary_is_at_the_top_or_in_one_folder() {
        assert!(is_the_binary("layover.exe", "layover.exe"));
        assert!(is_the_binary(
            "layover-cli-x86_64-unknown-linux-gnu/layover",
            "layover"
        ));
        assert!(!is_the_binary("a/b/layover", "layover"));
        assert!(!is_the_binary("layover-old", "layover"));
    }
}
