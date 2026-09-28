//! The MCP configuration a run's CLI is pointed at: Layover's own server, and every server the
//! agent declares.
//!
//! # Why one file holds all of them
//!
//! An agent's `[agents.<name>.mcp.<server>]` entries are part of its definition — a telemetry
//! agent's Kusto endpoint, a publisher's issue tracker — and the only channel Layover has into a
//! CLI's MCP setup is the one configuration it hands the run. A server declared in `layover.toml`
//! and left out of that file is a server the agent does not have, while `validate` accepts it and
//! the book says it works. So every declared server is written, in the runner's dialect, beside
//! Layover's own.
//!
//! # Why no credential value is written
//!
//! The file sits in the run's Hangar under `.layover/`, and `env_from` exists precisely so that a
//! secret never sits in a file. The values are already in the child's environment — the Tower puts
//! them there — so the file only has to say *which* variable a server reads:
//!
//! | Dialect | Forwarded by name as |
//! |---|---|
//! | `claude_json` | `"env": { "NAME": "${NAME}" }`, expanded from the CLI's own environment by Claude Code and Copilot CLI alike |
//! | `codex_toml` | `env_vars = ["NAME"]`, which Codex forwards from its environment without a value |
//!
//! Codex also needs no token for Layover's server: `bearer_token_env_var` names the variable the
//! run token is already in.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use layover_core::mcp::{McpServer, McpTransport};
use serde_json::{Map, Value, json};

use crate::tokens::TOKEN_VAR;

/// The name Layover's own server is written under.
///
/// Reserved: `validate` refuses an agent server by this name, because writing it would replace
/// the one server every run needs to send, report and ask for help.
pub const LAYOVER_SERVER: &str = "layover";

/// Writes the MCP configuration a runner's flag points at, returning where.
///
/// `servers` are the agent's declared servers. An unknown dialect gets the JSON shape rather than
/// a refusal: an unknown dialect is a factory that will not start, and the common shape is far
/// more likely to work than nothing at all.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_config(
    hangar: &Path,
    format: &str,
    endpoint: &str,
    token: &str,
    servers: &BTreeMap<String, McpServer>,
) -> std::io::Result<PathBuf> {
    let (name, body) = match format {
        "codex_toml" => ("mcp.toml", codex_toml(endpoint, servers)),
        _ => ("mcp.json", claude_json(endpoint, token, servers)),
    };

    let path = hangar.join(name);
    std::fs::write(&path, body)?;
    Ok(path)
}

/// The shape Claude Code and Copilot CLI read: a map of server name to how to reach it.
///
/// `type` is written for every entry. Copilot infers it, but Claude Code treats an entry without
/// one as stdio and then refuses it for having a `url`.
fn claude_json(endpoint: &str, token: &str, servers: &BTreeMap<String, McpServer>) -> String {
    let mut entries = Map::new();

    for (name, server) in servers {
        if name == LAYOVER_SERVER {
            continue;
        }

        let entry = match server.transport() {
            Ok(McpTransport::Stdio { command }) => {
                let Some((program, args)) = command.split_first() else {
                    continue;
                };
                let mut entry = json!({ "type": "stdio", "command": program, "args": args });
                let env = claude_env(server);
                if !env.is_empty() {
                    entry["env"] = Value::Object(env);
                }
                entry
            }
            Ok(McpTransport::Http { url }) => json!({ "type": "http", "url": url }),
            // Already an error at load; a server that cannot be described is left out rather
            // than written half-formed for the CLI to reject the whole file over.
            Err(_) => continue,
        };

        entries.insert(name.clone(), entry);
    }

    entries.insert(
        LAYOVER_SERVER.to_owned(),
        json!({
            "type": "http",
            "url": endpoint,
            "headers": { "Authorization": format!("Bearer {token}") },
        }),
    );

    serde_json::to_string_pretty(&json!({ "mcpServers": entries })).unwrap_or_default()
}

/// A stdio server's environment: its literal values, and a `${NAME}` reference for each name it
/// forwards.
fn claude_env(server: &McpServer) -> Map<String, Value> {
    let mut env: Map<String, Value> = server
        .env
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();

    for name in &server.env_from {
        env.insert(name.clone(), Value::String(format!("${{{name}}}")));
    }

    env
}

/// One `[mcp_servers.<name>]` table per server, the shape Codex keeps in its `config.toml`.
fn codex_toml(endpoint: &str, servers: &BTreeMap<String, McpServer>) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "[mcp_servers.{LAYOVER_SERVER}]");
    let _ = writeln!(out, "url = {}", quoted(endpoint));
    let _ = writeln!(out, "bearer_token_env_var = {}", quoted(TOKEN_VAR));

    for (name, server) in servers {
        if name == LAYOVER_SERVER {
            continue;
        }

        match server.transport() {
            Ok(McpTransport::Stdio { command }) => {
                let Some((program, args)) = command.split_first() else {
                    continue;
                };
                let _ = writeln!(out, "\n[mcp_servers.{name}]");
                let _ = writeln!(out, "command = {}", quoted(program));
                let _ = writeln!(out, "args = {}", array(args));
                if !server.env_from.is_empty() {
                    let _ = writeln!(out, "env_vars = {}", array(&server.env_from));
                }
                if !server.env.is_empty() {
                    let pairs = server
                        .env
                        .iter()
                        .map(|(key, value)| format!("{} = {}", quoted(key), quoted(value)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "env = {{ {pairs} }}");
                }
            }
            Ok(McpTransport::Http { url }) => {
                let _ = writeln!(out, "\n[mcp_servers.{name}]");
                let _ = writeln!(out, "url = {}", quoted(url));
            }
            Err(_) => {}
        }
    }

    out
}

/// A TOML basic string.
///
/// JSON's string escapes are a subset of TOML's basic-string escapes, so a JSON-encoded string is
/// a valid TOML one — which saves carrying a TOML serialiser for four kinds of value.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A TOML array of basic strings.
fn array(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|item| quoted(item))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-mcpcfg-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn servers(toml_text: &str) -> BTreeMap<String, McpServer> {
        toml::from_str(toml_text).expect("servers parse")
    }

    fn kusto() -> BTreeMap<String, McpServer> {
        servers(
            r#"
            [kusto]
            command  = ["agency", "mcp", "kusto", "--known-services", "[{\"uri\":\"x\"}]"]
            env      = { KUSTO_CLUSTER = "ic3-aria-eus2" }
            env_from = ["AZURE_CLIENT_SECRET"]

            [ado]
            url = "https://dev.azure.com/mcp/"
            "#,
        )
    }

    fn written(temp: &Temp, format: &str, servers: &BTreeMap<String, McpServer>) -> String {
        let path = write_config(
            &temp.0,
            format,
            "http://127.0.0.1:7878/mcp",
            "lvt_abc",
            servers,
        )
        .expect("writes");
        std::fs::read_to_string(path).expect("reads")
    }

    #[test]
    fn layovers_own_server_carries_the_endpoint_and_the_token() {
        let temp = Temp::new("own");
        let text = written(&temp, "claude_json", &BTreeMap::new());
        let config: Value = serde_json::from_str(&text).expect("JSON");

        let layover = &config["mcpServers"]["layover"];
        assert_eq!(layover["url"], "http://127.0.0.1:7878/mcp");
        assert_eq!(layover["headers"]["Authorization"], "Bearer lvt_abc");
        assert_eq!(
            layover["type"], "http",
            "Claude Code reads an entry without a type as stdio and refuses its url"
        );
    }

    #[test]
    fn a_stdio_server_is_written_with_its_command_split_and_its_arguments_intact() {
        let temp = Temp::new("stdio");
        let config: Value =
            serde_json::from_str(&written(&temp, "claude_json", &kusto())).expect("JSON");

        let kusto = &config["mcpServers"]["kusto"];
        assert_eq!(kusto["type"], "stdio");
        assert_eq!(kusto["command"], "agency");
        assert_eq!(
            kusto["args"],
            json!(["mcp", "kusto", "--known-services", "[{\"uri\":\"x\"}]"])
        );
        assert_eq!(kusto["env"]["KUSTO_CLUSTER"], "ic3-aria-eus2");
        assert_eq!(config["mcpServers"]["ado"]["type"], "http");
        assert_eq!(
            config["mcpServers"]["ado"]["url"],
            "https://dev.azure.com/mcp/"
        );
    }

    #[test]
    fn a_forwarded_credential_is_a_reference_to_the_environment_not_its_value() {
        let temp = Temp::new("reference");
        let config: Value =
            serde_json::from_str(&written(&temp, "claude_json", &kusto())).expect("JSON");

        assert_eq!(
            config["mcpServers"]["kusto"]["env"]["AZURE_CLIENT_SECRET"],
            "${AZURE_CLIENT_SECRET}"
        );
    }

    #[test]
    fn codex_gets_one_table_per_server_and_no_token_at_all() {
        let temp = Temp::new("codex");
        let text = written(&temp, "codex_toml", &kusto());
        let config: toml::Table = toml::from_str(&text).expect("valid TOML");
        let servers = config["mcp_servers"].as_table().expect("servers");

        assert_eq!(
            servers["layover"]["bearer_token_env_var"].as_str(),
            Some(TOKEN_VAR),
            "the run token is already in the child's environment"
        );
        assert!(!text.contains("lvt_abc"), "{text}");
        assert_eq!(servers["kusto"]["command"].as_str(), Some("agency"));
        assert_eq!(
            servers["kusto"]["args"].as_array().map(Vec::len),
            Some(4),
            "{text}"
        );
        assert_eq!(
            servers["kusto"]["env_vars"]
                .as_array()
                .and_then(|v| v[0].as_str()),
            Some("AZURE_CLIENT_SECRET")
        );
        assert_eq!(
            servers["kusto"]["env"]["KUSTO_CLUSTER"].as_str(),
            Some("ic3-aria-eus2")
        );
        assert_eq!(
            servers["ado"]["url"].as_str(),
            Some("https://dev.azure.com/mcp/")
        );
        assert!(
            temp.0.join("mcp.toml").exists(),
            "named for what it holds, as the book shows"
        );
    }

    #[test]
    fn a_server_named_layover_cannot_replace_layovers_own() {
        // Refused by `validate`, and ignored here as well: the run would otherwise lose the one
        // server it needs to send, report and ask for help.
        let temp = Temp::new("reserved");
        let impostor = servers(
            r#"
            [layover]
            url = "https://elsewhere.invalid/mcp"
            "#,
        );
        let config: Value =
            serde_json::from_str(&written(&temp, "claude_json", &impostor)).expect("JSON");

        assert_eq!(
            config["mcpServers"]["layover"]["url"],
            "http://127.0.0.1:7878/mcp"
        );
    }

    #[test]
    fn an_unknown_dialect_falls_back_rather_than_refusing() {
        let temp = Temp::new("unknown");
        let text = written(&temp, "something_new", &kusto());

        assert!(text.contains("mcpServers"), "{text}");
        assert!(text.contains("kusto"), "{text}");
    }
}
