//! Where releases come from: GitHub, or a mirror named by `LAYOVER_UPDATE_SOURCE`.
//!
//! # Why the download URL and not the API
//!
//! `api.github.com` answers sixty unauthenticated requests an hour per address, and behind a
//! corporate NAT that budget is shared with the whole building. The newest stable release's
//! manifest is at `releases/latest/download/dist-manifest.json`, a redirect github.com serves
//! without that limit. The API is asked only for what has no such URL: the newest pre-release.
//!
//! A mirror is laid out as GitHub's releases are — `<base>/latest/download/<file>`,
//! `<base>/download/<tag>/<file>` — with the API's release list at `<base>/releases.json`. It exists
//! for tests and for sites that cannot reach GitHub; what it serves is verified as GitHub's would be.

use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use crate::Channel;
use crate::manifest::Manifest;
use crate::version::Version;

/// Layover's releases on GitHub.
pub const GITHUB_RELEASES: &str = "https://github.com/KotkaZ/layover-project/releases";

/// The API's list of them, newest first.
pub const GITHUB_LISTING: &str =
    "https://api.github.com/repos/KotkaZ/layover-project/releases?per_page=30";

/// The largest manifest read. A real one is about 30 KB.
const MAX_MANIFEST: u64 = 2 * 1024 * 1024;

/// Where releases are fetched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    releases: String,
    listing: String,
    token: Option<String>,
}

/// What asking for the newest release found.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// A release, and when it was published where that is known.
    Found(Found),
    /// The same manifest as last time, by its entity tag.
    Unchanged,
}

/// A release's manifest, as fetched.
#[derive(Debug, Clone)]
pub struct Found {
    /// The manifest.
    pub manifest: Manifest,
    /// When the release was published: the manifest's upload time, or the API's date.
    pub published_at: Option<Timestamp>,
    /// What to send next time to be told nothing changed.
    pub etag: Option<Etag>,
}

/// An entity tag, and the address it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Etag {
    /// The address asked.
    pub url: String,
    /// What it answered.
    pub value: String,
}

/// Why a release could not be found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// Nothing answered: offline, a proxy, a TLS failure, a timeout.
    #[error("could not reach {url}: {reason}")]
    Unreachable {
        /// What was asked.
        url: String,
        /// What went wrong.
        reason: String,
    },
    /// Something answered, with an error.
    #[error("{url} answered HTTP {status}")]
    Status {
        /// What was asked.
        url: String,
        /// The status.
        status: u16,
    },
    /// What answered was not what was asked for.
    #[error("{url} did not return what was expected: {reason}")]
    Unreadable {
        /// What was asked.
        url: String,
        /// Why it could not be read.
        reason: String,
    },
    /// The list of releases had none this channel takes.
    #[error("no release was found on the {0} channel")]
    NoRelease(Channel),
}

/// One release in the API's list.
#[derive(Debug, Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: Option<Timestamp>,
}

impl Source {
    /// GitHub, with a token for the API where one is given.
    #[must_use]
    pub fn github(token: Option<String>) -> Self {
        Self {
            releases: GITHUB_RELEASES.to_owned(),
            listing: GITHUB_LISTING.to_owned(),
            token,
        }
    }

    /// A mirror at `base`, laid out as GitHub's releases are.
    #[must_use]
    pub fn mirror(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            releases: base.to_owned(),
            listing: format!("{base}/releases.json"),
            token: None,
        }
    }

    /// Where a person reads about the release tagged `tag`.
    #[must_use]
    pub fn page_for(&self, tag: &str) -> String {
        format!("{}/tag/{tag}", self.releases)
    }

    /// The address of `file` in the release tagged `tag`.
    #[must_use]
    pub fn file_in(&self, tag: &str, file: &str) -> String {
        format!("{}/download/{tag}/{file}", self.releases)
    }

    /// The newest release on `channel`.
    ///
    /// `cached` is the entity tag of the last answer, which a stable check sends so an unchanged
    /// manifest is not downloaded again.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] saying what could not be reached or read.
    pub fn newest(&self, channel: Channel, cached: Option<&Etag>) -> Result<Fetched, SourceError> {
        match channel {
            Channel::Stable => {
                let url = format!("{}/latest/download/dist-manifest.json", self.releases);
                Self::manifest(&url, cached.filter(|etag| etag.url == url), None)
            }
            Channel::Prerelease => {
                let (tag, published_at) = self.newest_listed()?;
                let url = self.file_in(&tag, "dist-manifest.json");
                Self::manifest(&url, None, published_at)
            }
        }
    }

    /// The manifest of the release tagged `tag`, for what it carries.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] saying what could not be reached or read.
    pub fn manifest_of(&self, tag: &str) -> Result<Manifest, SourceError> {
        match Self::manifest(&self.file_in(tag, "dist-manifest.json"), None, None)? {
            Fetched::Found(found) => Ok(found.manifest),
            Fetched::Unchanged => unreachable!("no entity tag was sent"),
        }
    }

    /// The newest release in the list, pre-releases included, and when it was published.
    fn newest_listed(&self) -> Result<(String, Option<Timestamp>), SourceError> {
        let mut request = agent()
            .get(&self.listing)
            .header("Accept", "application/vnd.github+json");
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request
            .call()
            .map_err(|error| unreachable(&self.listing, &error))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(SourceError::Status {
                url: self.listing.clone(),
                status,
            });
        }
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_MANIFEST)
            .read_to_string()
            .map_err(|error| unreachable(&self.listing, &error))?;
        let listed: Vec<Listed> =
            serde_json::from_str(&text).map_err(|error| SourceError::Unreadable {
                url: self.listing.clone(),
                reason: error.to_string(),
            })?;
        listed
            .into_iter()
            .filter(|release| !release.draft)
            .filter_map(|release| {
                let version: Version = release.tag_name.parse().ok()?;
                Some((version, release.tag_name, release.published_at))
            })
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, tag, published_at)| (tag, published_at))
            .ok_or(SourceError::NoRelease(Channel::Prerelease))
    }

    fn manifest(
        url: &str,
        cached: Option<&Etag>,
        published_at: Option<Timestamp>,
    ) -> Result<Fetched, SourceError> {
        let mut request = agent().get(url);
        if let Some(etag) = cached {
            request = request.header("If-None-Match", &etag.value);
        }
        let mut response = request.call().map_err(|error| unreachable(url, &error))?;
        let status = response.status().as_u16();
        if status == 304 && cached.is_some() {
            return Ok(Fetched::Unchanged);
        }
        if status != 200 {
            return Err(SourceError::Status {
                url: url.to_owned(),
                status,
            });
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let etag = header("etag").map(|value| Etag {
            url: url.to_owned(),
            value,
        });
        let uploaded = header("last-modified").and_then(|text| http_date(&text));
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_MANIFEST)
            .read_to_string()
            .map_err(|error| unreachable(url, &error))?;
        let manifest = Manifest::parse(&text).map_err(|error| SourceError::Unreadable {
            url: url.to_owned(),
            reason: error.to_string(),
        })?;
        if manifest.version().is_none() {
            return Err(SourceError::Unreadable {
                url: url.to_owned(),
                reason: "the manifest names no version".to_owned(),
            });
        }
        Ok(Fetched::Found(Found {
            manifest,
            published_at: published_at.or(uploaded),
            etag,
        }))
    }
}

/// An HTTP client that trusts what the machine trusts and goes through the proxy it is told to.
fn agent() -> Agent {
    agent_for(Duration::from_mins(1))
}

/// The same, given `limit` for a whole request: longer for an archive than for a manifest.
pub(crate) fn agent_for(limit: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(limit))
        .proxy(ureq::Proxy::try_from_env())
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent(concat!("layover/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) fn unreachable(url: &str, error: &ureq::Error) -> SourceError {
    SourceError::Unreachable {
        url: url.to_owned(),
        reason: error.to_string(),
    }
}

/// Reads an HTTP date, `Wed, 14 Oct 2026 13:50:10 GMT`.
fn http_date(text: &str) -> Option<Timestamp> {
    jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(text)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_http_date_is_read() {
        assert_eq!(
            http_date("Wed, 14 Oct 2026 13:50:10 GMT"),
            Some("2026-10-14T13:50:10Z".parse().expect("valid"))
        );
        assert_eq!(http_date("yesterday"), None);
    }

    #[test]
    fn a_mirror_is_laid_out_as_github_is() {
        let mirror = Source::mirror("http://127.0.0.1:9/releases/");
        assert_eq!(
            mirror.file_in("v1.15.0", "dist-manifest.json"),
            "http://127.0.0.1:9/releases/download/v1.15.0/dist-manifest.json"
        );
        assert_eq!(
            Source::github(None).page_for("v1.15.0"),
            "https://github.com/KotkaZ/layover-project/releases/tag/v1.15.0"
        );
    }
}
