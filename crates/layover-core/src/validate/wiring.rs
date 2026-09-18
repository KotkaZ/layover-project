//! Checks about the MCP servers agents reach, and about workspace isolation.

use crate::agent::Access;
use crate::config::Config;
use crate::graph::RouteGraph;
use crate::mcp::{McpTransport, looks_like_a_secret};

use super::Diagnostic;

pub(super) fn check_mcp_and_workspaces(config: &Config, found: &mut Vec<Diagnostic>) {
    check_mcp_servers(config, found);
    check_parallel_instances_are_isolated(config, found);
    check_agent_work_dirs(config, found);
}

fn check_mcp_servers(config: &Config, found: &mut Vec<Diagnostic>) {
    for (agent, definition) in &config.agents {
        for (server, spec) in &definition.mcp {
            let at = format!("agent `{agent}` MCP server `{server}`");

            if !is_identifier(server) {
                found.push(Diagnostic::error(format!(
                    "{at} is not a usable name; use letters, digits, hyphens and underscores"
                )));
            }

            match spec.transport() {
                Err(reason) => found.push(Diagnostic::error(format!("{at} {reason}"))),
                Ok(McpTransport::Stdio { command: [] }) => {
                    found.push(Diagnostic::error(format!("{at} has an empty `command`")));
                }
                Ok(McpTransport::Stdio { .. }) => {}
                Ok(McpTransport::Http { url }) => check_url(&at, url, found),
            }

            // The whole reason `env_from` exists. A literal here goes into git.
            for name in spec.env.keys() {
                if looks_like_a_secret(name) {
                    found.push(Diagnostic::error(format!(
                        "{at} sets `{name}` literally in `env`, and that name looks like a \
                         credential; move it to `env_from = [\"{name}\"]` so the value comes from \
                         the Tower's environment instead of this file"
                    )));
                }
            }

            for name in &spec.env_from {
                if name.trim().is_empty() {
                    found.push(Diagnostic::error(format!(
                        "{at} has an empty name in `env_from`"
                    )));
                }
            }
        }
    }
}

fn check_url(at: &str, url: &str, found: &mut Vec<Diagnostic>) {
    let local = url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]");

    if url.starts_with("https://") || local {
        return;
    }

    if url.starts_with("http://") {
        found.push(Diagnostic::warning(format!(
            "{at} uses plain HTTP to a non-local address; whatever it sends, including anything \
             forwarded through `env_from`, crosses the network in the clear"
        )));
    } else {
        found.push(Diagnostic::error(format!(
            "{at} has a `url` that is neither http nor https"
        )));
    }
}

/// Warns when a pipeline that can overlap itself would share a working directory.
///
/// A scheduled pipeline with `overlap = "allow"` fires whether or not the previous instance has
/// finished, so two instances can be live at once with nobody watching. If they share `work_dir`
/// and any reachable agent is `read-write`, they edit the same files at the same time — which
/// fails in the way hardest to notice: plausible output built from two unrelated changes.
///
/// A pipeline left on the default `overlap = "skip"` is **not** flagged, because it cannot reach
/// this state: the Tower misses the tick rather than starting a second copy. Warning about it
/// anyway would be a warning that is not true, and a validator that cries wolf is one people stop
/// reading.
///
/// Manual pipelines are deliberately not flagged either. A human choosing to start a second
/// instance knows they did. `workspace = "per-itinerary"` is still the right answer when you want
/// one instance per pull request; it is documented rather than nagged about.
fn check_parallel_instances_are_isolated(config: &Config, found: &mut Vec<Diagnostic>) {
    let graph = RouteGraph::from_config(config);

    for (name, pipeline) in config.scheduled_pipelines() {
        if pipeline.workspace.is_isolated() || !pipeline.allows_overlap() {
            continue;
        }

        let writers: Vec<String> = graph
            .reachable_from([&pipeline.entry])
            .into_iter()
            .filter(|agent| {
                config
                    .agents
                    .get(agent)
                    .is_some_and(|a| a.access == Access::ReadWrite)
            })
            .map(|agent| format!("`{agent}`"))
            .collect();

        if writers.is_empty() {
            continue;
        }

        found.push(Diagnostic::warning(format!(
            "pipeline `{name}` sets `overlap = \"allow\"`, reaches read-write \
             agent(s) {} and uses `workspace = \"shared\"`; two instances would edit the same \
             files at once. Set `workspace = \"per-itinerary\"` to give each its own worktree",
            writers.join(", ")
        )));
    }
}

fn check_agent_work_dirs(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        let Some(dir) = &agent.work_dir else {
            continue;
        };

        if dir.as_os_str().is_empty() {
            found.push(Diagnostic::error(format!(
                "agent `{name}` has an empty `work_dir`"
            )));
        }

        // A per-agent directory is outside whatever isolation a pipeline arranges, so a writer
        // pointed at one is shared across every itinerary whatever the pipeline says.
        if agent.access == Access::ReadWrite {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` is read-write and has its own `work_dir`, which sits outside any \
                 per-itinerary isolation; every instance of every pipeline writes to it"
            )));
        }
    }
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse, warnings};
    use crate::validate::validate;

    const WRITER: &str = r#"
        [agents.analyst]
        runner = "claude"
        description = "analyses"
        prompt = "analyse"

        [agents.developer]
        runner = "claude"
        description = "builds"
        prompt = "build"

        [[routes]]
        from = "analyst"
        to = "developer"
    "#;

    #[test]
    fn a_literal_credential_is_refused() {
        let config = parse(
            r#"
            [agents.kusto]
            runner = "claude"
            description = "queries telemetry"
            prompt = "query"
            entry = true

            [agents.kusto.mcp.kusto]
            command = ["agency", "mcp", "kusto"]
            env = { KUSTO_CLUSTER = "eus2", AZURE_CLIENT_SECRET = "hunter2" }
            "#,
        );

        let errors = errors(&config);
        assert_mentions(&errors, "AZURE_CLIENT_SECRET");
        assert_mentions(&errors, "env_from");
        assert!(
            !errors.iter().any(|m| m.contains("hunter2")),
            "the diagnostic must not echo the value it is telling you to remove: {errors:?}"
        );
    }

    #[test]
    fn forwarding_a_credential_by_name_is_accepted() {
        let config = parse(
            r#"
            [agents.kusto]
            runner = "claude"
            description = "queries telemetry"
            prompt = "query"
            entry = true
            access = "read-only"

            [agents.kusto.mcp.kusto]
            command = ["agency", "mcp", "kusto"]
            env = { KUSTO_CLUSTER = "eus2" }
            env_from = ["AZURE_CLIENT_SECRET"]
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn an_empty_command_is_refused() {
        let config = parse(
            r#"
            [agents.kusto]
            runner = "claude"
            description = "queries"
            prompt = "query"
            entry = true

            [agents.kusto.mcp.broken]
            command = []
            "#,
        );

        assert_mentions(&errors(&config), "empty `command`");
    }

    #[test]
    fn plain_http_to_a_remote_server_warns() {
        let config = parse(
            r#"
            [agents.kusto]
            runner = "claude"
            description = "queries"
            prompt = "query"
            entry = true
            access = "read-only"

            [agents.kusto.mcp.remote]
            url = "http://mcp.example.com/"
            "#,
        );

        assert_mentions(&warnings(&config), "in the clear");
    }

    #[test]
    fn plain_http_to_loopback_is_fine() {
        let config = parse(
            r#"
            [agents.kusto]
            runner = "claude"
            description = "queries"
            prompt = "query"
            entry = true
            access = "read-only"

            [agents.kusto.mcp.local]
            url = "http://127.0.0.1:7878/mcp"
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_shared_workspace_with_a_writer_warns_only_when_overlap_is_allowed() {
        let config = parse(&format!(
            r#"
            [reserve]
            fuel_usd = 500.0

            {WRITER}

            [pipelines.development]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            overlap = "allow"
            "#
        ));

        assert_mentions(
            &warnings(&config),
            "two instances would edit the same files",
        );
    }

    #[test]
    fn the_default_schedule_cannot_overlap_so_nothing_is_said() {
        // The Tower skips a tick whose previous wave is still going, so two instances never
        // exist. A warning about a state that cannot be reached is a warning people learn to
        // ignore.
        let config = parse(&format!(
            r#"
            [reserve]
            fuel_usd = 500.0

            {WRITER}

            [pipelines.development]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            "#
        ));

        assert!(
            !warnings(&config)
                .iter()
                .any(|w| w.contains("two instances would edit the same files")),
            "a skip-by-default schedule cannot overlap itself"
        );
    }

    #[test]
    fn per_itinerary_isolation_makes_parallel_instances_safe() {
        let config = parse(&format!(
            r#"
            [reserve]
            fuel_usd = 500.0

            {WRITER}

            [pipelines.development]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            workspace = "per-itinerary"
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_read_only_pipeline_needs_no_isolation() {
        // Nothing writes, so nothing collides.
        let config = parse(
            r#"
            [agents.scanner]
            runner = "claude"
            description = "scans"
            prompt = "scan"
            access = "read-only"

            [pipelines.review-bot]
            entry = "scanner"
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_read_write_agent_with_its_own_work_dir_warns() {
        let config = parse(
            r#"
            [agents.publisher]
            runner = "claude"
            description = "publishes"
            prompt = "publish"
            entry = true
            work_dir = "../other-repo"
            recovery = "manual"

            [pipelines.publish]
            entry = "publisher"
            workspace = "per-itinerary"
            "#,
        );

        assert_mentions(&warnings(&config), "outside any per-itinerary isolation");
    }
}
