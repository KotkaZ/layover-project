//! Generating the HTTP server from `api/openapi.yaml`.
//!
//! The specification is the contract. This module turns it into Rust, and `cargo xtask verify`
//! fails when the committed output is stale, so the server cannot drift away from the document
//! that describes it.
//!
//! # The supported subset
//!
//! Only what Layover's own API needs:
//!
//! - named schemas that are objects with `properties`, or strings with `enum`
//! - property types: `string`, `boolean`, `integer` (int32/int64), `number`, `array`, `$ref`, and
//!   `object` with `additionalProperties` for maps
//! - optionality via `required` or `type: [T, "null"]`
//! - parameters in `path` and `query`
//! - `application/json` request bodies
//! - `application/json` or `text/event-stream` success responses
//!
//! Anything else is an error naming the construct. That is the point: a specification the
//! generator silently ignored would be a specification nobody could trust.

mod model;
mod schema;
mod server;

use std::fmt::Write as _;
use std::path::Path;

pub use model::Document;

/// Where the specification lives, relative to the repository root.
pub const SPEC_PATH: &str = "api/openapi.yaml";

/// Where the generated server lives, relative to the repository root.
pub const OUTPUT_PATH: &str = "crates/layover-http/src/generated.rs";

/// Why generation failed.
#[derive(Debug)]
pub enum Error {
    /// The specification could not be read.
    Read(std::io::Error),
    /// The specification was not valid YAML, or used fields outside the subset.
    Parse(String),
    /// The specification used a construct the generator cannot express.
    Unsupported {
        /// What was found, and where.
        what: String,
    },
    /// The generated source could not be formatted, or `rustfmt` rejected it.
    Format(String),
    /// The committed output no longer matches the specification.
    Stale,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "could not read {SPEC_PATH}: {error}"),
            Self::Parse(reason) => write!(f, "could not parse {SPEC_PATH}: {reason}"),
            Self::Unsupported { what } => write!(
                f,
                "{SPEC_PATH} uses something the generator does not support: {what}\n\
                 Either change the specification or widen the subset in xtask/src/openapi/."
            ),
            Self::Format(reason) => write!(f, "{reason}"),
            Self::Stale => write!(
                f,
                "{OUTPUT_PATH} is out of date with {SPEC_PATH}.\n\
                 Run `cargo xtask generate-api` and commit the result."
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Whether to write the generated file or only check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Write the file if it differs.
    Write,
    /// Fail if the file differs, changing nothing.
    Check,
}

/// Generates the HTTP server, writing or checking depending on `mode`.
///
/// # Errors
///
/// Returns an error when the specification cannot be read, parsed or expressed, or — in
/// [`Mode::Check`] — when the committed output is stale.
pub fn generate(root: &Path, mode: Mode) -> Result<String, Error> {
    let document = load(root)?;
    let rendered = rustfmt(&render(&document)?)?;
    let target = root.join(OUTPUT_PATH);

    let current = std::fs::read_to_string(&target).ok();
    if current.as_deref() == Some(rendered.as_str()) {
        return Ok(format!("{OUTPUT_PATH} is up to date"));
    }

    match mode {
        Mode::Check => Err(Error::Stale),
        Mode::Write => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(Error::Read)?;
            }
            std::fs::write(&target, &rendered).map_err(Error::Read)?;
            Ok(format!("wrote {OUTPUT_PATH}"))
        }
    }
}

/// Formats generated source with `rustfmt`.
///
/// The output has to be byte-identical to what `cargo fmt` would produce, or `verify` would flag
/// the file as stale immediately after formatting it. Emitting pre-formatted code is simpler than
/// teaching either tool to ignore the other.
///
/// # Errors
///
/// Returns [`Error::Format`] when `rustfmt` cannot be run or refuses the source. Both are worth
/// stopping for: `rustfmt` refusing means the generator emitted Rust that does not parse, and
/// reporting that as a successful write pushes the real failure into a later step that cannot
/// explain it. A missing `rustfmt` would instead make a correctly formatted committed file look
/// stale, which is just as confusing.
fn rustfmt(source: &str) -> Result<String, Error> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::Format(format!("could not run `rustfmt`: {error}")))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Format("`rustfmt` refused its input".to_owned()))?;
        stdin
            .write_all(source.as_bytes())
            .map_err(|error| Error::Format(format!("could not write to `rustfmt`: {error}")))?;
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|error| Error::Format(format!("`rustfmt` did not finish: {error}")))?;

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Format(format!(
            "`rustfmt` rejected the generated source, so the generator emitted Rust that does \
             not parse:\n{}",
            reason.trim()
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|_| Error::Format("`rustfmt` returned invalid UTF-8".to_owned()))
}

/// Reads and parses the specification.
///
/// # Errors
///
/// Returns [`Error::Read`] or [`Error::Parse`] when the document is missing or malformed.
pub fn load(root: &Path) -> Result<Document, Error> {
    let text = std::fs::read_to_string(root.join(SPEC_PATH)).map_err(Error::Read)?;
    serde_yaml_bw::from_str(&text).map_err(|error| Error::Parse(error.to_string()))
}

/// Renders the whole generated module.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] when the specification uses a construct outside the subset.
pub fn render(document: &Document) -> Result<String, Error> {
    // Bodies are rendered first so the header can import only what they actually use: a
    // specification with no streaming endpoint must not emit an unused import, because
    // `verify` denies warnings and such a specification could then never pass.
    let mut schemas = String::new();
    schema::emit_schemas(&document.components.schemas, &mut schemas)?;

    let mut server = String::new();
    server::emit_server(document, &mut server)?;

    let mut out = String::new();

    let _ = writeln!(
        out,
        "//! The Layover HTTP surface, generated from `{SPEC_PATH}`."
    );
    out.push_str(
        "//!\n\
         //! @generated by `cargo xtask generate-api`. Do not edit by hand: `cargo xtask verify`\n\
         //! regenerates this file and fails if the result differs, so an edit here is reverted\n\
         //! rather than kept. Change the specification instead.\n",
    );
    let _ = writeln!(
        out,
        "//!\n//! Source: {} v{}",
        document.info.title, document.info.version
    );
    out.push_str("\n#![allow(clippy::too_many_lines)]\n\n");
    out.push_str("use axum::response::IntoResponse as _;\n");
    out.push_str("use serde::{Deserialize, Serialize};\n\n");
    if server.contains("EventStream") {
        out.push_str("use crate::EventStream;\n\n");
    }

    out.push_str(&schemas);
    out.push_str(&server);

    Ok(out)
}

/// Emits a doc comment, indented by `indent` spaces.
fn docs(text: Option<&str>, indent: usize, out: &mut String) {
    let Some(text) = text else {
        return;
    };

    let pad = " ".repeat(indent);
    for line in text.trim_end().lines() {
        if line.trim().is_empty() {
            let _ = writeln!(out, "{pad}///");
        } else {
            let _ = writeln!(out, "{pad}/// {line}");
        }
    }
}

/// Escapes a string for use inside a Rust `"..."` literal.
///
/// Values here come from the specification rather than from a user at runtime, so this is
/// robustness rather than a security boundary — but a stray quote in an enum value would emit
/// source that does not compile, and the error would point at generated code instead of at the
/// line that caused it.
fn string_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());

    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }

    out
}

/// `sendFlight` becomes `SendFlight`; `read-only` becomes `ReadOnly`.
fn type_name(raw: &str) -> String {
    let mut out = String::new();
    let mut upper = true;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }

    out
}

/// `sendFlight` becomes `send_flight`.
fn method_name(raw: &str) -> String {
    let mut out = String::new();

    for (index, ch) in raw.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_are_pascal_case() {
        assert_eq!(type_name("sendFlight"), "SendFlight");
        assert_eq!(type_name("read-only"), "ReadOnly");
        assert_eq!(type_name("ok"), "Ok");
        assert_eq!(type_name("event-stream"), "EventStream");
    }

    #[test]
    fn method_names_are_snake_case() {
        assert_eq!(method_name("sendFlight"), "send_flight");
        assert_eq!(method_name("listRuns"), "list_runs");
        assert_eq!(method_name("getHealth"), "get_health");
    }

    #[test]
    fn doc_comments_preserve_blank_lines() {
        let mut out = String::new();
        docs(Some("first\n\nsecond"), 4, &mut out);

        assert_eq!(out, "    /// first\n    ///\n    /// second\n");
    }

    /// Builds a minimal document around the given `components/schemas` YAML.
    fn document_with(schemas: &str) -> Result<Document, String> {
        let text = format!(
            "openapi: 3.2.0\ninfo:\n  title: t\n  version: 0.0.0\ncomponents:\n  schemas:\n{schemas}"
        );
        serde_yaml_bw::from_str(&text).map_err(|error| error.to_string())
    }

    fn render_error(schemas: &str) -> String {
        let document = document_with(schemas).expect("test document parses");
        match render(&document) {
            Ok(text) => panic!("expected a refusal, generated:\n{text}"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn two_schema_names_that_collide_are_refused() {
        // `fooBar` and `foo-bar` both render as `FooBar`. Emitting both would produce a duplicate
        // definition, and the failure would point at generated code instead of the specification.
        let message = render_error(
            "    fooBar:\n      type: string\n      description: one\n      enum: [a]\n\
             \x20   foo-bar:\n      type: string\n      description: two\n      enum: [b]\n",
        );

        assert!(message.contains("both become the Rust type"), "{message}");
    }

    #[test]
    fn two_enum_values_that_collide_are_refused() {
        let message = render_error(
            "    Access:\n      type: string\n      description: d\n      enum: [read-only, readOnly]\n",
        );

        assert!(message.contains("both become the variant"), "{message}");
    }

    #[test]
    fn an_undescribed_schema_is_refused() {
        let message = render_error("    Thing:\n      type: string\n      enum: [a]\n");

        assert!(message.contains("no `description`"), "{message}");
    }

    #[test]
    fn a_field_named_after_a_keyword_becomes_a_raw_identifier() {
        let document = document_with(
            "    Thing:\n      type: object\n      description: d\n      required: [type]\n\
             \x20     properties:\n        type:\n          type: string\n          description: k\n",
        )
        .expect("parses");

        let rendered = render(&document).expect("a keyword field is usable as a raw identifier");
        assert!(rendered.contains("pub r#type: String"), "{rendered}");
    }

    #[test]
    fn a_field_that_is_not_an_identifier_is_refused() {
        let message = render_error(
            "    Thing:\n      type: object\n      description: d\n      required: [\"a-b\"]\n\
             \x20     properties:\n        a-b:\n          type: string\n          description: k\n",
        );

        assert!(
            message.contains("not a usable Rust field name"),
            "{message}"
        );
    }

    #[test]
    fn an_inline_object_is_refused_rather_than_silently_flattened() {
        let message = render_error(
            "    Thing:\n      type: object\n      description: d\n      required: [nested]\n\
             \x20     properties:\n        nested:\n          type: object\n          description: k\n\
             \x20         properties:\n            x:\n              type: string\n              description: v\n",
        );

        assert!(message.contains("inline object"), "{message}");
    }

    #[test]
    fn an_inline_enum_is_refused_rather_than_widened_to_a_string() {
        // Emitting `String` would silently drop the constraint the specification promised.
        let message = render_error(
            "    Thing:\n      type: object\n      description: d\n      required: [kind]\n\
             \x20     properties:\n        kind:\n          type: string\n          description: k\n\
             \x20         enum: [a, b]\n",
        );

        assert!(message.contains("inline `enum`"), "{message}");
    }

    #[test]
    fn string_literals_are_escaped() {
        assert_eq!(string_literal(r#"a"b"#), r#"a\"b"#);
        assert_eq!(string_literal(r"a\b"), r"a\\b");
        assert_eq!(string_literal("a\nb"), "a\\nb");
        assert_eq!(string_literal("plain"), "plain");
    }

    #[test]
    fn a_specification_with_no_streaming_endpoint_imports_nothing_unused() {
        // `verify` denies warnings, so an unconditional `use crate::EventStream;` would make a
        // perfectly valid JSON-only specification impossible to ship.
        let text = "openapi: 3.2.0\n\
            info:\n  title: t\n  version: 0.0.0\n\
            paths:\n  /health:\n    get:\n      operationId: getHealth\n      summary: s\n\
            \x20     responses:\n        \"200\":\n          description: ok\n\
            \x20         content:\n            application/json:\n              schema:\n\
            \x20               $ref: \"#/components/schemas/Health\"\n\
            components:\n  schemas:\n    Health:\n      type: object\n      description: d\n\
            \x20     required: [ok]\n      properties:\n        ok:\n          type: boolean\n\
            \x20         description: o\n";

        let document: Document = serde_yaml_bw::from_str(text).expect("parses");
        let rendered = render(&document).expect("renders");

        assert!(!rendered.contains("use crate::EventStream;"), "{rendered}");
    }
}
