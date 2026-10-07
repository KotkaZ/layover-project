//! How a runner invokes an agent CLI: its command, the placeholders substituted into it, and how
//! it is told where Layover's MCP server is.

use serde::Deserialize;

/// How Layover passes its MCP endpoint to a runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpWiring {
    /// Command-line flag carrying the MCP configuration.
    pub flag: String,
    /// Configuration dialect the runner expects.
    pub format: String,
    /// Prepended to the path the flag carries.
    ///
    /// Copilot CLI's `--additional-mcp-config` takes *either* a JSON string or a file path, and
    /// tells the two apart by a leading `@`. Without it the path is read as JSON, which fails as
    /// a parse error about the factory's own configuration rather than anything recognisable.
    ///
    /// Empty for CLIs that take a plain path, which is most of them.
    #[serde(default)]
    pub prefix: String,
}

impl McpWiring {
    /// The argument that follows [`Self::flag`].
    #[must_use]
    pub fn argument(&self, path: &str) -> String {
        format!("{}{path}", self.prefix)
    }
}

/// How to invoke a particular headless agent CLI.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    /// Command and arguments.
    ///
    /// `{prompt}` substitutes the **path** to the agent's composed instructions, which the Tower
    /// writes into the run's Hangar before spawning. It is not the instructions themselves.
    ///
    /// That distinction is the whole design. A prompt is passed on **stdin**, never on the
    /// command line: Windows caps a command line at 32,767 characters, and real agent prompts go
    /// well past it — a sibling project's review agent composes to roughly 98 KB, three times
    /// over, and its ordinary developer agent to 34 KB. Inlining the prompt would work in every
    /// test written against a small fixture and fail on the first agent worth running.
    ///
    /// So most runners need no placeholder at all. It exists for CLIs that accept a file of
    /// instructions as a flag; those that do not get the instructions prepended to stdin.
    pub command: Vec<String>,
    /// How this runner is told where Layover's MCP server is.
    #[serde(default)]
    pub mcp: Option<McpWiring>,
}

impl Runner {
    /// The placeholder substituted with the path to the composed instructions.
    pub const PROMPT_PATH: &'static str = "{prompt}";

    /// The placeholder substituted with the agent's model.
    ///
    /// Every supported CLI spells its model flag differently — `--model`, `-m`, a config key — so
    /// the spelling stays in the runner command, which is already the one place that knows how to
    /// invoke a given CLI. The alternative, a `model_flag` field, would put half of an invocation
    /// in one place and half in another.
    pub const MODEL: &'static str = "{model}";

    /// The placeholder substituted with this runner's [`McpWiring::flag`] and the path to the
    /// generated MCP configuration — two arguments, not one.
    ///
    /// Optional. A command that does not contain it gets the pair appended at the end, which is
    /// what `claude` and `copilot` want. It exists for commands that end in a positional argument
    /// — `codex exec … -` reads the prompt from stdin and must stay last — where appending would
    /// put a flag after the thing it has to precede.
    pub const MCP: &'static str = "{mcp}";

    /// Returns `true` when this runner wants the instructions as a file it is handed.
    ///
    /// When `false`, the Tower prepends them to the stdin payload instead.
    #[must_use]
    pub fn takes_prompt_path(&self) -> bool {
        self.command
            .iter()
            .any(|arg| arg.contains(Self::PROMPT_PATH))
    }

    /// Returns `true` when this runner can carry an agent's `model`.
    ///
    /// An agent that declares a model whose runner cannot carry it is a silent no-op: the run
    /// happens, on whichever model the CLI defaults to, and nothing says the declaration was
    /// ignored. Validation warns about it rather than letting it pass.
    #[must_use]
    pub fn takes_model(&self) -> bool {
        self.command.iter().any(|arg| arg.contains(Self::MODEL))
    }

    /// Builds the command line for one run.
    ///
    /// Substitution is textual and deliberately so: a placeholder sits inside an argument like
    /// `--model={model}` as readily as it stands alone, and the operator writes whichever their
    /// CLI expects.
    ///
    /// An argument that is *only* a `{model}` placeholder disappears when no model is set, rather
    /// than becoming an empty argument — an empty string in `argv` is not nothing, and several
    /// CLIs treat it as a positional.
    #[must_use]
    pub fn invocation(&self, prompt_path: Option<&str>, model: Option<&str>) -> Vec<String> {
        self.invocation_with_mcp(prompt_path, model, None)
    }

    /// The command to run, with every placeholder resolved.
    ///
    /// `mcp_config` is the path to the file [`McpWiring::format`] describes. When the command
    /// names [`Self::MCP`] the flag and path replace it there; otherwise they are appended, which
    /// is the right answer for every CLI that does not end in a positional argument.
    #[must_use]
    pub fn invocation_with_mcp(
        &self,
        prompt_path: Option<&str>,
        model: Option<&str>,
        mcp_config: Option<&str>,
    ) -> Vec<String> {
        let wiring = self.mcp.as_ref().zip(mcp_config);
        let mut out = Vec::with_capacity(self.command.len() + 2);

        for arg in &self.command {
            if arg == Self::MODEL && model.is_none() {
                continue;
            }

            if arg == Self::MCP {
                if let Some((mcp, path)) = wiring {
                    out.push(mcp.flag.clone());
                    out.push(mcp.argument(path));
                }
                // Dropped when there is nothing to wire: an unresolved placeholder reaching a CLI
                // becomes an argument it does not understand.
                continue;
            }

            let mut rendered = arg.clone();
            if let Some(path) = prompt_path {
                rendered = rendered.replace(Self::PROMPT_PATH, path);
            }
            if let Some(model) = model {
                rendered = rendered.replace(Self::MODEL, model);
            }

            out.push(rendered);
        }

        if let Some((mcp, path)) = wiring
            && !self.command.iter().any(|arg| arg == Self::MCP)
        {
            out.push(mcp.flag.clone());
            out.push(mcp.argument(path));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Runner;

    fn runner(args: &[&str]) -> Runner {
        toml::from_str(&format!(
            "command = [{}]",
            args.iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .expect("parses")
    }

    #[test]
    fn a_model_placeholder_is_substituted_wherever_it_sits() {
        // Some CLIs take `--model x`, some take `--model=x`. The operator writes whichever theirs
        // wants, so substitution has to be textual rather than positional.
        let separate = runner(&["claude", "-p", "--model", "{model}"]);
        assert_eq!(
            separate.invocation(None, Some("claude-opus-5")),
            ["claude", "-p", "--model", "claude-opus-5"]
        );

        let joined = runner(&["codex", "exec", "--model={model}"]);
        assert_eq!(
            joined.invocation(None, Some("gpt-5.4")),
            ["codex", "exec", "--model=gpt-5.4"]
        );
    }

    #[test]
    fn a_bare_model_placeholder_disappears_when_no_model_is_set() {
        // An empty string in argv is not nothing; several CLIs read it as a positional argument.
        let r = runner(&["claude", "-p", "{model}"]);
        assert_eq!(r.invocation(None, None), ["claude", "-p"]);
    }

    fn mcp_runner(args: &[&str], flag: &str) -> Runner {
        toml::from_str(&format!(
            "command = [{}]\nmcp = {{ flag = \"{flag}\", format = \"claude_json\" }}",
            args.iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .expect("parses")
    }

    #[test]
    fn mcp_wiring_is_appended_when_the_command_does_not_place_it() {
        // What `claude` and `copilot` want, and what every existing factory file relies on.
        let r = mcp_runner(&["copilot", "--allow-all-tools"], "--mcp-config");
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            [
                "copilot",
                "--allow-all-tools",
                "--mcp-config",
                "/h/mcp.json"
            ]
        );
    }

    #[test]
    fn a_prefix_is_prepended_to_the_path_rather_than_passed_separately() {
        // Copilot CLI's `--additional-mcp-config` takes either a JSON string or a file path and
        // tells them apart by a leading `@`. Passed as its own argument the `@` would be a second
        // value the flag never sees; without it the path is parsed as JSON and the run dies
        // complaining about the factory's own configuration.
        let runner: Runner = toml::from_str(
            r#"command = ["copilot", "--allow-all-tools"]
mcp = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }"#,
        )
        .expect("parses");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            [
                "copilot",
                "--allow-all-tools",
                "--additional-mcp-config",
                "@/h/mcp.json"
            ]
        );
    }

    #[test]
    fn a_prefix_applies_where_the_command_places_the_wiring_too() {
        // Both branches render the argument, and only one of them having the prefix would be a
        // factory that works until somebody adds `{mcp}` to keep a positional argument last.
        let runner: Runner = toml::from_str(
            r#"command = ["agent", "{mcp}", "-"]
mcp = { flag = "--cfg", format = "claude_json", prefix = "@" }"#,
        )
        .expect("parses");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["agent", "--cfg", "@/h/mcp.json", "-"]
        );
    }

    #[test]
    fn a_runner_without_a_prefix_still_gets_a_bare_path() {
        let runner = mcp_runner(&["claude", "-p"], "--mcp-config");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["claude", "-p", "--mcp-config", "/h/mcp.json"]
        );
    }

    #[test]
    fn mcp_wiring_goes_where_the_command_puts_it_when_it_says() {
        // `codex exec … -` reads the prompt from stdin and the `-` has to stay last, so appending
        // would put the flag after the argument it must precede.
        let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.toml")),
            ["codex", "exec", "-c", "/h/mcp.toml", "-"]
        );
    }

    #[test]
    fn an_mcp_placeholder_disappears_when_there_is_nothing_to_wire() {
        // An unresolved placeholder reaching a CLI becomes an argument it does not understand.
        let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
        assert_eq!(
            r.invocation_with_mcp(None, None, None),
            ["codex", "exec", "-"]
        );
    }

    #[test]
    fn a_runner_with_no_mcp_block_is_wired_to_nothing_even_if_a_path_exists() {
        // The factory always has a config file to offer; only the runner knows whether its CLI
        // can be told about one.
        let r = runner(&["echo", "hello"]);
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["echo", "hello"]
        );
    }

    #[test]
    fn the_prompt_path_is_substituted_independently_of_the_model() {
        let r = runner(&["agent", "--file", "{prompt}", "--model", "{model}"]);
        assert_eq!(
            r.invocation(Some("/run/prompt.md"), Some("m1")),
            ["agent", "--file", "/run/prompt.md", "--model", "m1"]
        );
    }

    #[test]
    fn a_runner_without_placeholders_is_passed_through_untouched() {
        let r = runner(&["copilot", "--allow-all-tools"]);
        assert_eq!(
            r.invocation(Some("/x"), Some("m")),
            ["copilot", "--allow-all-tools"]
        );
        assert!(!r.takes_model());
        assert!(!r.takes_prompt_path());
    }

    #[test]
    fn declaring_a_model_a_runner_cannot_carry_is_a_warning() {
        let config: crate::config::Config = toml::from_str(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
model = "claude-opus-5"
entry = true
"#,
        )
        .expect("parses");

        let said: Vec<_> = crate::validate::validate(&config)
            .iter()
            .map(|d| d.message.clone())
            .collect();

        assert!(
            said.iter().any(|m| m.contains("no `{model}` placeholder")),
            "{said:?}"
        );
    }
}
