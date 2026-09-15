//! The single verification entrypoint for this repository.
//!
//! `cargo xtask verify` is the only definition of done. CI runs this exact command, so a local
//! pass is a CI pass.

use std::path::Path;
use std::process::{Command, ExitCode};

use crate::{docs, openapi};

/// Runs every gate, in the order that fails fastest.
pub fn run(root: &Path) -> ExitCode {
    // Cheap, in-process checks first: no point compiling if the generated code is stale.
    println!("\n=== generated ===");
    match openapi::generate(root, openapi::Mode::Check) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("{error}");
            eprintln!("\nverify failed at `generated`");
            return ExitCode::FAILURE;
        }
    }

    println!("\n=== docs ===");
    match docs::check(root) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("{error}");
            eprintln!("\nverify failed at `docs`");
            return ExitCode::FAILURE;
        }
    }

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
        command.args(args).current_dir(root);
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
