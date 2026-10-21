//! Repository automation.
//!
//! `cargo xtask verify` is the only definition of done. CI runs this exact command, so a local
//! pass is a CI pass.
//!
//! The other tasks exist so that things which *could* drift cannot: the HTTP server is generated
//! from `api/openapi.yaml`, and the documentation is checked for links that no longer resolve.
//! Both run inside `verify`, which is what makes them binding rather than advisory.

mod anchors;
mod api_docs;
mod book;
mod changelog;
mod docs;
mod openapi;
mod verify;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let root = repository_root();

    match task.as_deref() {
        Some("verify") => verify::run(&root),
        Some("generate-api") => match openapi::generate(&root, openapi::Mode::Write) {
            Ok(report) => {
                println!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Some("book") => match book_arguments() {
            Ok((tree, git_ref)) => match book::generate(tree.as_deref().unwrap_or(&root), &git_ref)
            {
                Ok(report) => {
                    println!("{report}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("{error}");
                usage();
                ExitCode::FAILURE
            }
        },
        Some("docs") => match docs::check(&root) {
            Ok(report) => {
                println!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
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
    eprintln!("usage: cargo xtask <task>");
    eprintln!();
    eprintln!("  verify        format, lint, generated-code freshness, docs, test, doc build");
    eprintln!("  generate-api  regenerate the HTTP server from api/openapi.yaml");
    eprintln!("  docs          check documentation links and examples");
    eprintln!("  book [--root DIR] [--ref REF]");
    eprintln!("                write book/src/design/ from docs/, linking code at REF (main)");
}

/// `--root` and `--ref` for `book`: which tree to write the design pages into, and which commit
/// links to code should name.
fn book_arguments() -> Result<(Option<PathBuf>, String), String> {
    let mut tree = None;
    let mut git_ref = "main".to_owned();
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("`{arg}` needs a value"))?;
        match arg.as_str() {
            "--root" => tree = Some(PathBuf::from(value)),
            "--ref" => git_ref = value,
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok((tree, git_ref))
}

/// The repository root, found from this crate's manifest rather than the working directory.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask always lives one level below the repository root")
        .to_path_buf()
}
