//! The `layover` binary.
//!
//! What exists today is everything that happens *before* the first process is spawned: loading a
//! factory definition, checking it, and showing what it would do. `layover run` is deliberately
//! absent rather than stubbed, because a command that pretends to start a factory is worse than
//! one that says it cannot.

mod commands;

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

    let result = match cli.command {
        Command::Validate { strict } => commands::validate_config(&cli.config, strict),
        Command::Explain => commands::explain(&cli.config),
        Command::Prompt {
            agent,
            pipeline,
            flags,
        } => commands::prompt(&cli.config, &agent, pipeline.as_deref(), &flags),
        Command::Autostart { output, show } => {
            commands::autostart(&cli.config, output.as_deref(), show)
        }
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
