//! Compiles in the target triple, which Cargo tells a build script and nothing else.
//!
//! An update must be the archive built for the machine it runs on, and the only reliable source of
//! that is the triple this binary was compiled for.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=LAYOVER_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
