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
            eprintln!("run `cargo xtask generate-api` to regenerate it from api/openapi.yaml");
            return ExitCode::FAILURE;
        }
    }

    println!("\n=== docs ===");
    match docs::check(root) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("{error}");
            eprintln!("\nverify failed at `docs`");
            eprintln!(
                "a link no longer resolves, or a version disagrees; the report above says which"
            );
            return ExitCode::FAILURE;
        }
    }

    // `--locked` throughout: without it a manifest change can quietly update Cargo.lock in CI's
    // own checkout and pass, leaving the lockfile the release is built from uncommitted.
    let steps: [(&str, &[&str]); 4] = [
        ("format", &["fmt", "--all", "--check"]),
        (
            "lint",
            &[
                "clippy",
                "--locked",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        ("test", &["test", "--locked", "--workspace"]),
        ("doc", &["doc", "--locked", "--workspace", "--no-deps"]),
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
                if name == "format" {
                    eprintln!("run `cargo fmt --all` to fix it");
                }
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
