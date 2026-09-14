//! The single verification entrypoint for this repository.
//!
//! `cargo xtask verify` is the only definition of done. CI runs this exact command, so a local
//! pass is a CI pass. It deliberately has no dependencies beyond the standard library: an agent
//! must be able to run it on a clean checkout without installing anything.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let task = std::env::args().nth(1);

    match task.as_deref() {
        Some("verify") => verify(),
        Some(other) => {
            eprintln!("unknown task `{other}`");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("usage: cargo xtask verify");
}

fn verify() -> ExitCode {
    let steps: [(&str, &[&str]); 4] = [
        ("format", &["fmt", "--all", "--check"]),
        (
            "lint",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        ("test", &["test", "--workspace"]),
        ("doc", &["doc", "--workspace", "--no-deps"]),
    ];

    for (name, args) in steps {
        println!("\n=== {name} ===");

        let mut command = Command::new(env!("CARGO"));
        command.args(args);
        if name == "doc" {
            command.env("RUSTDOCFLAGS", "-D warnings");
        }

        match command.status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("\nverify failed at `{name}` ({status})");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("\ncould not run `cargo {}`: {error}", args.join(" "));
                return ExitCode::FAILURE;
            }
        }
    }

    println!("\nverify passed");
    ExitCode::SUCCESS
}
