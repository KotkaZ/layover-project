//! What Layover supplies for a CLI it knows, so a runner says only what is particular to it.
//!
//! Every Copilot runner in a real factory began with the same eleven arguments: the three
//! placeholders, `--no-ask-user` because nobody is there to answer, `--output-format json` because
//! that is the only output a cost is read from, and the three `--allow-all-*` because an unattended
//! run cannot answer a permission prompt either. Five runners repeated them and differed only in
//! what each denied, which is how a copy-and-edit dropped two deny rules from every runner for a
//! week. A preset says those once, here, and a runner's `args` say what is left: what it takes
//! away.
//!
//! Only what a CLI *requires* to run unattended and be read afterwards is supplied, plus — for
//! Copilot — permission to use its tools, whose prompts an unattended run could never answer.
//! Nothing that narrows what an agent may do is ever supplied, because that is the part that must
//! be written down where it can be reviewed.

use serde::Deserialize;

use super::McpWiring;

/// A CLI Layover knows how to run unattended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cli {
    /// GitHub Copilot CLI, `copilot`.
    Copilot,
    /// Claude Code, `claude`.
    Claude,
}

/// Copilot CLI's arguments for an unattended, priced, MCP-wired run.
///
/// `--name` makes the run's session carry the name its chain was given, so it can be found among
/// the operator's own Copilot sessions. The `--allow-all-*` grant what an unattended run cannot be
/// asked for; a runner's `--deny-tool` and `--deny-url` rules take precedence over them.
const COPILOT: &[&str] = &[
    "--model={model}",
    "--reasoning-effort={effort}",
    "--context={context}",
    "--name={name}",
    "--allow-all-tools",
    "--allow-all-paths",
    "--allow-all-urls",
    "--no-ask-user",
    "--output-format",
    "json",
];

/// Claude Code's arguments for an unattended, priced, MCP-wired run.
///
/// `-p` with no prompt reads it from stdin. `stream-json` is the output a cost is read from, and in
/// print mode Claude Code refuses it without `--verbose`. Claude Code takes no reasoning effort or
/// context tier on its command line, so an agent on this preset that sets either is warned about.
/// Its permission mode is left to the runner: nothing here can confirm against a real Claude Code
/// what a bypass would grant.
const CLAUDE: &[&str] = &[
    "-p",
    "--model={model}",
    "--output-format",
    "stream-json",
    "--verbose",
];

impl Cli {
    /// The name `layover.toml` uses for it.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Copilot => "copilot",
            Self::Claude => "claude",
        }
    }

    /// The program to run.
    #[must_use]
    pub fn program(self) -> &'static str {
        self.slug()
    }

    /// The arguments the preset supplies, after the program and before the runner's own.
    #[must_use]
    pub fn supplied(self) -> &'static [&'static str] {
        match self {
            Self::Copilot => COPILOT,
            Self::Claude => CLAUDE,
        }
    }

    /// How the CLI is told where Layover's MCP server is.
    #[must_use]
    pub fn wiring(self) -> McpWiring {
        McpWiring {
            flag: self.wiring_flag().to_owned(),
            format: "claude_json".to_owned(),
            prefix: match self {
                Self::Copilot => "@".to_owned(),
                Self::Claude => String::new(),
            },
        }
    }

    /// The whole command a runner on this preset runs before any agent's own arguments: the
    /// program, what the preset supplies, the runner's `args`, and where an agent's go.
    #[must_use]
    pub fn template(self, runner_args: &[String]) -> Vec<String> {
        std::iter::once(self.program())
            .chain(self.supplied().iter().copied())
            .map(ToOwned::to_owned)
            .chain(runner_args.iter().cloned())
            .chain(std::iter::once(super::Runner::ARGS.to_owned()))
            .collect()
    }

    /// The option `arg` sets, when the preset already sets it — `--no-ask-user`, or `--model` for
    /// `--model=gpt-5.4` — so validation can say a runner or agent repeats it.
    #[must_use]
    pub fn already_supplies(self, arg: &str) -> Option<&'static str> {
        let option = option_name(arg)?;
        self.supplied()
            .iter()
            .filter_map(|supplied| option_name(supplied))
            .find(|supplied| *supplied == option)
            .or_else(|| (self.wiring_flag() == option).then(|| self.wiring_flag()))
    }

    /// The flag that carries the MCP configuration.
    fn wiring_flag(self) -> &'static str {
        match self {
            Self::Copilot => "--additional-mcp-config",
            Self::Claude => "--mcp-config",
        }
    }
}

impl std::fmt::Display for Cli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// The option an argument sets — `--model` for `--model=x` and for `--model` — or `None` for an
/// argument that is not an option.
pub(super) fn option_name(arg: &str) -> Option<&str> {
    if arg.len() < 2 || !arg.starts_with('-') || arg == "--" {
        return None;
    }
    Some(arg.split_once('=').map_or(arg, |(option, _)| option))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copilot_preset_is_everything_an_unattended_run_needs_and_nothing_it_denies() {
        let line = Cli::Copilot.template(&["--deny-tool=shell(git push)".to_owned()]);

        assert_eq!(line[0], "copilot");
        for needed in [
            "--model={model}",
            "--reasoning-effort={effort}",
            "--context={context}",
            "--no-ask-user",
            "--allow-all-tools",
        ] {
            assert!(line.iter().any(|arg| arg == needed), "{needed}: {line:?}");
        }
        let json = line.iter().position(|arg| arg == "--output-format");
        assert_eq!(
            json.and_then(|at| line.get(at + 1)).map(String::as_str),
            Some("json")
        );
        assert_eq!(
            &line[line.len() - 2..],
            ["--deny-tool=shell(git push)", "{args}"],
            "the runner's own, then the agent's"
        );
        assert!(
            Cli::Copilot
                .supplied()
                .iter()
                .all(|arg| !arg.starts_with("--deny") && !arg.starts_with("--disable")),
            "nothing that takes something away is the preset's to decide"
        );
    }

    #[test]
    fn a_repeated_preset_option_is_recognised_in_either_form() {
        assert_eq!(
            Cli::Copilot.already_supplies("--no-ask-user"),
            Some("--no-ask-user")
        );
        assert_eq!(
            Cli::Copilot.already_supplies("--model=gpt-5.4"),
            Some("--model")
        );
        assert_eq!(
            Cli::Copilot.already_supplies("--additional-mcp-config"),
            Some("--additional-mcp-config")
        );
        assert_eq!(
            Cli::Copilot.already_supplies("--deny-tool=shell(gh:*)"),
            None
        );
        assert_eq!(Cli::Copilot.already_supplies("json"), None);
        assert_eq!(Cli::Claude.already_supplies("--verbose"), Some("--verbose"));
    }
}
