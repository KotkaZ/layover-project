//! The `layover` binary.
//!
//! What exists today is everything that happens *before* the first process is spawned: loading a
//! factory definition, checking it, and showing what it would do. `layover run` is deliberately
//! absent rather than stubbed, because a command that pretends to start a factory is worse than
//! one that says it cannot.

mod commands;
mod doctor;
mod mcp;
mod tower;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Run a lights-out agent factory.
#[derive(Debug, Parser)]
#[command(name = "layover", version, about, long_about = None)]
struct Cli {
    /// Path to the factory definition.
    #[arg(
        long,
        short,
        global = true,
        default_value = "layover.toml",
        value_name = "FILE"
    )]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check a factory definition and report everything wrong with it.
    ///
    /// Exits non-zero when anything would block startup. Warnings are printed but do not fail,
    /// unless `--strict` is given.
    Validate {
        /// Treat warnings as failures.
        #[arg(long)]
        strict: bool,
    },

    /// Describe the factory: its agents, its pipelines and its route map.
    Explain,

    /// Render an agent's prompt exactly as a run would receive it.
    ///
    /// This is how you find out what a conditional prompt actually composes to without spending
    /// a real invocation to see it.
    Prompt {
        /// The agent whose prompt to render.
        agent: String,

        /// The pipeline whose flag defaults to use.
        #[arg(long, value_name = "NAME")]
        pipeline: Option<String>,

        /// Override a flag, as `name=true` or `name=false`. May be repeated.
        #[arg(long = "flag", value_name = "NAME=BOOL")]
        flags: Vec<String>,
    },

    /// Print the factory's route map as a diagram.
    ///
    /// Mermaid by default, which is for *portability*: paste it into a README and GitHub draws
    /// it. `--svg` prints the same graph as the dashboard draws it, laid out here rather than by
    /// a JavaScript library.
    Graph {
        /// Emit SVG instead of Mermaid source.
        #[arg(long)]
        svg: bool,
    },

    /// Run the factory and serve the dashboard.
    ///
    /// This is the lights-out command, and what `layover autostart` registers. It fires scheduled
    /// pipelines, runs what is queued, serves agents the MCP endpoint they call back into, and
    /// puts a dashboard over all of it.
    ///
    /// `--watch-only` leaves the running out and serves the dashboard alone, which is what you
    /// want when looking at a factory another process is already running. Two Towers over one
    /// factory directory would race for its queue.
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: String,

        /// Where run history lives. Defaults to `.layover/history` beside the configuration.
        #[arg(long, value_name = "DIR")]
        history: Option<PathBuf>,

        /// Serve the dashboard without running anything.
        #[arg(long)]
        watch_only: bool,

        /// Serve without a token, open to anything that can reach the port.
        ///
        /// A token is minted at startup and printed in the address by default. This turns that
        /// off, which is reasonable on a machine only you can reach and is not otherwise.
        #[arg(long)]
        no_auth: bool,
    },

    /// Run the queued work, once.
    ///
    /// Takes everything waiting in the queue and runs it to completion: each flight is authorised
    /// against the route map and the rails, spawned, watched, and written to history.
    ///
    /// Deliberately **not** a daemon yet. Nothing here routes a message from one agent to
    /// another, and there is no MCP server for them to talk through, so a factory drains what was
    /// asked for and stops. A command that looped forever would look like a working factory
    /// that never does anything.
    Run {
        /// Report what would happen without starting anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Report anything about a running factory that a person should look at.
    ///
    /// The failures that matter in a lights-out factory are the quiet ones: a chain that stalled
    /// with every run reporting success, a schedule that has not fired, a cost total built from
    /// runners that reported nothing. This finds them and says what to do.
    ///
    /// Exits non-zero when anything found would fail an unattended run, which makes it usable as
    /// the verdict at the end of a soak rather than a judgement call.
    Doctor {
        /// How far back to look.
        #[arg(long, default_value = "last_7d", value_name = "WINDOW")]
        window: String,
    },

    /// Write the file that starts Layover when you log in.
    ///
    /// A lights-out factory that stops at every reboot is not lights-out. This generates the
    /// platform's own artefact — a Scheduled Task, a launchd agent or a systemd user unit — and
    /// tells you the one command that registers it. It deliberately does not register it for
    /// you: that touches the machine, and you should see what is being installed first.
    Autostart {
        /// Where to write the generated file. Defaults to beside the configuration.
        #[arg(long, short, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Print the file instead of writing it.
        #[arg(long)]
        show: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Handled apart from the rest because its exit code carries the verdict rather than merely
    // whether the command worked. A check that always exits zero cannot be the thing a soak is
    // judged by.
    if let Command::Doctor { window } = &cli.command {
        return match commands::doctor(&cli.config, window) {
            Ok((output, healthy)) => {
                print!("{output}");
                if healthy {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let result = match cli.command {
        Command::Validate { strict } => commands::validate_config(&cli.config, strict),
        Command::Explain => commands::explain(&cli.config),
        Command::Graph { svg } => commands::graph(&cli.config, svg),
        Command::Prompt {
            agent,
            pipeline,
            flags,
        } => commands::prompt(&cli.config, &agent, pipeline.as_deref(), &flags),
        Command::Autostart { output, show } => {
            commands::autostart(&cli.config, output.as_deref(), show)
        }
        Command::Serve {
            addr,
            history,
            watch_only,
            no_auth,
        } => commands::serve(&cli.config, &addr, history.as_deref(), watch_only, no_auth),
        Command::Run { dry_run } => commands::run(&cli.config, dry_run),
        Command::Doctor { .. } => unreachable!("handled above"),
    };

    match result {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use layover_core::Autostart;

    /// The autostart entry invokes a subcommand by name, from a crate that cannot see this
    /// parser. Nothing else connects the two, so a rename here leaves a generated service that
    /// fails at every logon, reporting only to a Windows event log nobody reads.
    ///
    /// This asserts the actual property that matters: the command we tell the operating system to
    /// run is a command this binary accepts.
    #[test]
    fn the_autostart_command_is_one_the_cli_accepts() {
        let parsed = Cli::try_parse_from(["layover", Autostart::COMMAND, "--config", "f.toml"]);

        assert!(
            parsed.is_ok(),
            "`layover {}` is what every generated autostart entry runs, and this binary rejects \
             it: {}",
            Autostart::COMMAND,
            parsed.unwrap_err()
        );
    }

    #[test]
    fn the_parser_itself_is_valid() {
        Cli::command().debug_assert();
    }
}
