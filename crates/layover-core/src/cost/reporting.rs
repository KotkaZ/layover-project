//! Whether a runner's output can carry what its runs cost.
//!
//! Layover reads a run's cost from what the CLI prints, and each supported CLI prints it only in
//! one output mode: Copilot CLI its AI credits with `--output-format json`, Claude Code its dollars
//! with `--output-format json` or `stream-json`, and Codex no price at all — only token counts, with
//! `--json`, which a rate card can estimate. A runner started without that mode records every run
//! as reporting nothing, its spend reads as unknown, and Fuel and the Reserve never bind it.
//!
//! The CLI is found by name anywhere in the command, so a wrapper — `cmd /c copilot`,
//! `npx @github/copilot` — is recognised as readily as the bare binary. A command that names none
//! of them is not judged: a stand-in or another CLI may print whatever it likes, and Layover reads
//! it.

/// An agent CLI whose cost output Layover knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    /// GitHub Copilot CLI.
    Copilot,
    /// Claude Code.
    Claude,
    /// `OpenAI` Codex.
    Codex,
}

impl Cli {
    /// The CLI a command line runs, if it names one Layover knows.
    #[must_use]
    pub fn of(command: &[String]) -> Option<Self> {
        command.iter().find_map(|arg| {
            let file = arg.rsplit(['/', '\\']).next()?;
            let stem = file.split('.').next()?.to_ascii_lowercase();
            match stem.as_str() {
                "copilot" => Some(Self::Copilot),
                "claude" => Some(Self::Claude),
                "codex" => Some(Self::Codex),
                _ => None,
            }
        })
    }

    /// Its name, as a person would say it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Copilot => "Copilot CLI",
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// What a runner's output says about what its runs cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reporting {
    /// It prints dollars: Claude Code with JSON output.
    Dollars,
    /// It prints the Copilot AI credits a run used, which Layover prices.
    Credits,
    /// It prints token counts and no price: Codex with `--json`. Only a rate card can estimate it.
    Tokens,
    /// A CLI Layover knows, run without the output that would carry its cost.
    Nothing {
        /// Which CLI.
        cli: Cli,
        /// What to add to the command to have it print one.
        add: &'static str,
    },
    /// A command that names no CLI Layover knows. Whatever it prints is read; nothing is assumed.
    Unknown,
}

impl Reporting {
    /// What `command` will print about cost.
    #[must_use]
    pub fn of(command: &[String]) -> Self {
        let format = crate::model::value_of(command, "--output-format");
        match Cli::of(command) {
            Some(Cli::Copilot) if format.as_deref() == Some("json") => Self::Credits,
            Some(Cli::Claude) if matches!(format.as_deref(), Some("json" | "stream-json")) => {
                Self::Dollars
            }
            Some(Cli::Codex) if command.iter().any(|arg| arg == "--json") => Self::Tokens,
            Some(cli) => Self::Nothing {
                cli,
                add: match cli {
                    Cli::Copilot => "--output-format json",
                    Cli::Claude => "--output-format stream-json",
                    Cli::Codex => "--json",
                },
            },
            None => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(command: &[&str]) -> Reporting {
        Reporting::of(
            &command
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn each_cli_reports_its_cost_only_in_its_json_output() {
        assert_eq!(
            of(&["copilot", "--allow-all-tools", "--output-format", "json"]),
            Reporting::Credits
        );
        assert_eq!(
            of(&["claude", "-p", "--output-format", "stream-json"]),
            Reporting::Dollars
        );
        assert_eq!(
            of(&["claude", "-p", "--output-format=json"]),
            Reporting::Dollars
        );
        assert_eq!(of(&["codex", "exec", "--json", "-"]), Reporting::Tokens);
    }

    #[test]
    fn without_it_each_says_what_to_add() {
        assert_eq!(
            of(&["copilot", "--allow-all-tools"]),
            Reporting::Nothing {
                cli: Cli::Copilot,
                add: "--output-format json"
            }
        );
        assert_eq!(
            of(&["claude", "-p", "--output-format", "text"]),
            Reporting::Nothing {
                cli: Cli::Claude,
                add: "--output-format stream-json"
            }
        );
        assert_eq!(
            of(&["codex", "exec", "--model", "{model}", "-"]),
            Reporting::Nothing {
                cli: Cli::Codex,
                add: "--json"
            }
        );
    }

    #[test]
    fn a_wrapped_cli_is_recognised_and_a_model_named_after_one_is_not() {
        assert_eq!(
            of(&["cmd", "/c", "copilot.cmd", "--output-format", "json"]),
            Reporting::Credits
        );
        assert_eq!(
            Cli::of(&["npx".to_owned(), "@github/copilot".to_owned()]),
            Some(Cli::Copilot)
        );
        assert_eq!(
            Cli::of(&["C:\\tools\\claude.exe".to_owned()]),
            Some(Cli::Claude)
        );
        assert_eq!(
            Cli::of(&[
                "agent".to_owned(),
                "--model".to_owned(),
                "claude-opus-4.5".to_owned()
            ]),
            None
        );
    }

    #[test]
    fn a_command_that_names_no_known_cli_is_not_judged() {
        assert_eq!(of(&["python", "stand-in.py"]), Reporting::Unknown);
        assert_eq!(of(&[]), Reporting::Unknown);
    }
}
