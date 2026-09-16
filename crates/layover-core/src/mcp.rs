//! MCP servers an agent may reach.
//!
//! Layover is itself an MCP server — that is how agents send flights. This module is about the
//! *other* servers an agent needs to do its job: a telemetry agent that queries Kusto, a
//! publisher that talks to Azure DevOps. Without this, those have to be configured outside
//! Layover in each CLI's own settings, where the route map cannot see them and nothing validates
//! them.
//!
//! # Secrets do not live here
//!
//! `layover.toml` is a file people commit. Credentials reach servers through [`McpServer::env_from`],
//! which forwards named variables from the Tower's own environment, and validation refuses a
//! literal that looks like a credential. This is the same rule as everywhere else in Layover: keys
//! reach child processes through the environment, never through config.

use std::collections::BTreeMap;

use serde::Deserialize;

/// How Layover reaches an MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport<'a> {
    /// A subprocess speaking MCP over stdio.
    Stdio {
        /// Command and arguments.
        command: &'a [String],
    },
    /// An HTTP endpoint.
    Http {
        /// Where the server lives.
        url: &'a str,
    },
}

/// One MCP server, as declared for an agent.
///
/// Exactly one of `command` and `url` must be given. They are separate optional fields rather
/// than an untagged enum because `deny_unknown_fields` — which is what catches a typo like
/// `comand` — cannot be combined with a flattened enum, and silently accepting a misspelt field
/// would leave an agent without the server it thinks it has.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    /// Command and arguments, for a server spoken to over stdio.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Endpoint, for a server spoken to over HTTP.
    #[serde(default)]
    pub url: Option<String>,
    /// Non-secret environment variables, set literally.
    ///
    /// For values that are safe in a committed file: a cluster name, a region, a default
    /// database. Anything that authenticates belongs in [`McpServer::env_from`].
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Names of variables forwarded from the Tower's own environment.
    ///
    /// The Tower reads these at spawn time and passes them through. The value never appears in
    /// `layover.toml`, so the file stays committable.
    #[serde(default)]
    pub env_from: Vec<String>,
}

/// Why an MCP server declaration is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    /// Neither `command` nor `url` was given.
    #[error("declares neither `command` nor `url`")]
    Neither,
    /// Both were given, so which applies is undefined.
    #[error("declares both `command` and `url`; give exactly one")]
    Both,
}

impl McpServer {
    /// Returns how this server is reached.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when neither or both of `command` and `url` are set.
    pub fn transport(&self) -> Result<McpTransport<'_>, McpError> {
        match (self.command.as_deref(), self.url.as_deref()) {
            (Some(command), None) => Ok(McpTransport::Stdio { command }),
            (None, Some(url)) => Ok(McpTransport::Http { url }),
            (Some(_), Some(_)) => Err(McpError::Both),
            (None, None) => Err(McpError::Neither),
        }
    }
}

/// Environment variable names that almost always hold a credential.
const SECRET_HINTS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "APIKEY",
    "API_KEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
    "_PAT",
    "PAT_",
    "SESSION_KEY",
    "CLIENT_SECRET",
];

/// Returns `true` when a variable name suggests its value is a credential.
///
/// Deliberately name-based rather than value-based: guessing whether a *string* is a secret is
/// hopeless, while `AZURE_CLIENT_SECRET` announces itself. False positives cost an author one
/// line — move it to `env_from` — and a false negative costs a leaked key in git history.
#[must_use]
pub fn looks_like_a_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_HINTS.iter().any(|hint| upper.contains(hint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(body: &str) -> McpServer {
        toml::from_str(body).expect("server parses")
    }

    #[test]
    fn a_stdio_server_is_a_command() {
        let server = server(r#"command = ["agency", "mcp", "kusto"]"#);

        assert_eq!(
            server.transport(),
            Ok(McpTransport::Stdio {
                command: ["agency", "mcp", "kusto"].map(str::to_owned).as_slice()
            })
        );
    }

    #[test]
    fn an_http_server_is_a_url() {
        let server = server(r#"url = "https://api.example.com/mcp/""#);

        assert_eq!(
            server.transport(),
            Ok(McpTransport::Http {
                url: "https://api.example.com/mcp/"
            })
        );
    }

    #[test]
    fn non_secret_environment_is_kept_literal() {
        let server = server(
            r#"
            command = ["agency", "mcp", "kusto"]
            env = { KUSTO_CLUSTER = "ic3-aria-eus2", KUSTO_DATABASE = "Web Media Prod" }
            env_from = ["AZURE_TENANT_ID"]
            "#,
        );

        assert_eq!(server.env["KUSTO_CLUSTER"], "ic3-aria-eus2");
        assert_eq!(server.env_from, ["AZURE_TENANT_ID"]);
    }

    #[test]
    fn declaring_both_transports_is_refused() {
        let server = server(
            r#"
            command = ["x"]
            url = "https://example.com"
            "#,
        );

        assert_eq!(server.transport(), Err(McpError::Both));
    }

    #[test]
    fn a_server_that_is_neither_command_nor_url_is_rejected() {
        let server = server(r#"env = { A = "b" }"#);

        assert_eq!(server.transport(), Err(McpError::Neither));
    }

    #[test]
    fn a_typo_in_a_server_field_is_rejected() {
        assert!(toml::from_str::<McpServer>(r#"comand = ["x"]"#).is_err());
    }

    #[test]
    fn credential_shaped_names_are_recognised() {
        for name in [
            "GITHUB_TOKEN",
            "github_token",
            "AZURE_CLIENT_SECRET",
            "API_KEY",
            "apikey",
            "ADO_PAT_VALUE",
            "MY_PASSWORD",
            "AWS_ACCESS_KEY_ID",
            "SSH_PRIVATE_KEY",
        ] {
            assert!(looks_like_a_secret(name), "`{name}` should be flagged");
        }
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        for name in [
            "KUSTO_CLUSTER",
            "AZURE_TENANT_ID",
            "HOME",
            "RUST_LOG",
            "DATABASE",
            "REGION",
        ] {
            assert!(!looks_like_a_secret(name), "`{name}` should not be flagged");
        }
    }
}
