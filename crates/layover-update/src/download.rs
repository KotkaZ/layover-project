//! Fetching a release's archive, and proving it is the file the release lists.
//!
//! Streamed to disk with a hard cap — sixty-four megabytes, about twenty times Layover's largest
//! archive — and hashed as it arrives. The SHA-256 must equal both the manifest's and the `.sha256`
//! file published beside the archive; any disagreement refuses the archive and says both values.

use std::io::{Read, Write as _};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest as _, Sha256};

use crate::source::agent_for;

/// The largest archive downloaded.
pub const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;

/// How long one download may take, start to finish.
const LIMIT: Duration = Duration::from_mins(10);

/// Downloads `url` to `to`, refusing more than `cap` bytes, and returns its SHA-256 in hex.
///
/// Written beside `to` and renamed into place, so a download that fails leaves nothing that
/// looks finished.
///
/// # Errors
///
/// Returns why it could not be fetched or was too large.
pub fn fetch(url: &str, to: &Path, cap: u64) -> Result<String, String> {
    let mut response = agent_for(LIMIT)
        .get(url)
        .call()
        .map_err(|error| crate::source::unreachable(url, &error).to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("{url} answered HTTP {status}"));
    }
    let partial = to.with_extension(format!("partial-{}", std::process::id()));
    let result = (|| {
        let mut body = response
            .body_mut()
            .with_config()
            .limit(cap.saturating_add(1))
            .reader()
            .take(cap.saturating_add(1));
        let mut file = std::fs::File::create(&partial).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let read = body
                .read(&mut buffer)
                .map_err(|error| format!("the download of {url} broke off: {error}"))?;
            if read == 0 {
                break;
            }
            total += u64::try_from(read).unwrap_or(u64::MAX);
            if total > cap {
                return Err(format!("{url} is larger than {cap} bytes"));
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
        }
        file.sync_all().map_err(|error| error.to_string())?;
        Ok(hex(&hasher.finalize()))
    })();
    match result {
        Ok(digest) => {
            std::fs::rename(&partial, to).map_err(|error| error.to_string())?;
            Ok(digest)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

/// The digest a `.sha256` file at `url` gives: `<hex> *<name>` or `<hex>  <name>`.
///
/// # Errors
///
/// Returns why it could not be fetched or read.
pub fn sidecar(url: &str) -> Result<String, String> {
    let mut response = agent_for(Duration::from_mins(1))
        .get(url)
        .call()
        .map_err(|error| crate::source::unreachable(url, &error).to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("{url} answered HTTP {status}"));
    }
    let text = response
        .body_mut()
        .with_config()
        .limit(4096)
        .read_to_string()
        .map_err(|error| error.to_string())?;
    digest_in(&text).ok_or_else(|| format!("{url} holds no SHA-256"))
}

/// The SHA-256 at the start of a checksum file's text, lower-cased.
fn digest_in(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?;
    (first.len() == 64 && first.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| first.to_ascii_lowercase())
}

/// The SHA-256 of the file at `path`, in hex.
///
/// # Errors
///
/// Returns the I/O error.
pub fn digest_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checksum_file_is_read_as_dist_writes_it() {
        let digest = "1842a1c7cdbd5a4b9c2fd3215411a9263e5a00bc12175ec1648da1d42a955085";
        assert_eq!(
            digest_in(&format!(
                "{digest} *layover-cli-x86_64-pc-windows-msvc.zip\n"
            ))
            .as_deref(),
            Some(digest)
        );
        assert_eq!(
            digest_in(&format!("{}  layover\n", digest.to_ascii_uppercase())).as_deref(),
            Some(digest)
        );
        assert_eq!(digest_in("not a digest"), None);
        assert_eq!(digest_in(""), None);
    }

    #[test]
    fn a_files_digest_is_its_sha256() {
        let path = std::env::temp_dir().join(format!("layover-digest-{}", std::process::id()));
        std::fs::write(&path, b"abc").expect("writes");
        assert_eq!(
            digest_of(&path).expect("reads"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&path);
    }
}
