//! An agent's declared MCP servers reach the run, and their credentials do not reach the disk.
//!
//! `[agents.<name>.mcp.<server>]` validated, and the book said it worked, but the `mcp.json` a run
//! was handed named only Layover's own server. An agent declared with a Kusto server got no Kusto
//! tools, and nothing said so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_tower::Factory;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-mcpsrv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A factory whose one agent declares a stdio server and an HTTP server.
///
/// The stdio server forwards `PATH` by name. It stands in for a credential because it is set in
/// every environment a test runs in, and a test may not set a variable of its own: the value must
/// be something the process already holds, which is exactly the situation `env_from` is for.
fn factory(temp: &Temp, format: &str) -> Factory {
    let echo = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };

    let text = format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
timeout_sec = 30

[runners.shell]
command = {echo}
mcp = {{ flag = "--mcp-config", format = "{format}" }}

[agents.kusto]
prompt = "query"
entry = true

[agents.kusto.mcp.kusto]
command  = ["agency", "mcp", "kusto"]
env      = {{ KUSTO_CLUSTER = "ic3-aria-eus2" }}
env_from = ["PATH"]

[agents.kusto.mcp.docs]
url = "https://example.invalid/mcp/"
"#
    );

    let config: Config = toml::from_str(&text).expect("the fixture factory parses");
    Factory::new(config, &temp.0)
        .expect("opens")
        .serving_mcp("http://127.0.0.1:9/mcp")
}

fn run_once(factory: &Factory) {
    let drained = factory.drain(
        vec![Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("kusto"),
                "how many calls dropped",
                4,
            ),
            None,
            BTreeMap::new(),
        )],
        |_| {},
        |_, _| {},
    );
    assert_eq!(drained.ran, 1);
}

/// The MCP configuration the run was handed, whatever its dialect called it.
fn written(root: &Path) -> String {
    std::fs::read_dir(root.join(".layover").join("hangars").join("kusto"))
        .expect("a Hangar")
        .filter_map(Result::ok)
        .flat_map(|entry| [entry.path().join("mcp.json"), entry.path().join("mcp.toml")])
        .find(|path| path.exists())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .expect("an MCP configuration")
}

/// Every file under `.layover/`, read as text where it is text.
fn everything_under(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(everything_under(&path));
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            found.push((path, text));
        }
    }
    found
}

#[test]
fn every_declared_server_reaches_the_run_in_the_claude_dialect() {
    let temp = Temp::new("claude");
    let factory = factory(&temp, "claude_json");

    run_once(&factory);

    let config: serde_json::Value =
        serde_json::from_str(&written(&temp.0)).expect("the configuration is JSON");
    let servers = &config["mcpServers"];

    assert_eq!(servers["layover"]["type"], "http", "{config}");
    assert_eq!(servers["kusto"]["type"], "stdio", "{config}");
    assert_eq!(servers["kusto"]["command"], "agency", "{config}");
    assert_eq!(
        servers["kusto"]["args"],
        serde_json::json!(["mcp", "kusto"]),
        "{config}"
    );
    assert_eq!(
        servers["kusto"]["env"]["KUSTO_CLUSTER"], "ic3-aria-eus2",
        "{config}"
    );
    assert_eq!(servers["docs"]["type"], "http", "{config}");
    assert_eq!(
        servers["docs"]["url"], "https://example.invalid/mcp/",
        "{config}"
    );
}

#[test]
fn every_declared_server_reaches_the_run_in_the_codex_dialect() {
    let temp = Temp::new("codex");
    let factory = factory(&temp, "codex_toml");

    run_once(&factory);

    let text = written(&temp.0);
    let config: toml::Table = toml::from_str(&text).expect("the configuration is TOML");
    let servers = config["mcp_servers"]
        .as_table()
        .expect("a table of servers");

    assert!(servers.contains_key("layover"), "{text}");
    assert_eq!(
        servers["kusto"]["command"].as_str(),
        Some("agency"),
        "{text}"
    );
    assert_eq!(
        servers["docs"]["url"].as_str(),
        Some("https://example.invalid/mcp/"),
        "{text}"
    );
}

#[test]
fn a_forwarded_credential_is_named_in_the_configuration_and_never_written_down() {
    // `env_from` exists so a secret never sits in a committed file. Writing its value into the
    // run's configuration would move it from git history to a Hangar, served by nothing but read
    // by anything that can open the directory.
    let secret = std::env::var("PATH").expect("PATH is always set");

    for format in ["claude_json", "codex_toml"] {
        let temp = Temp::new(&format!("secret-{format}"));
        let factory = factory(&temp, format);

        run_once(&factory);

        let text = written(&temp.0);
        assert!(
            text.contains("PATH"),
            "{format}: the server is told which variable to read: {text}"
        );
        for (path, contents) in everything_under(&temp.0.join(".layover")) {
            assert!(
                !contents.contains(&secret),
                "{format}: {} holds a forwarded value",
                path.display()
            );
        }
    }
}
