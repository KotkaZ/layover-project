//! Turning `OpenAPI` schemas into Rust types.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::model::Schema;
use super::{Error, docs, string_literal, type_name};

/// Emits every named schema as a Rust struct or enum.
pub fn emit_schemas(schemas: &BTreeMap<String, Schema>, out: &mut String) -> Result<(), Error> {
    check_names_do_not_collide(schemas)?;

    for (name, schema) in schemas {
        emit_named(name, schema, out)?;
    }
    Ok(())
}

/// Refuses two schema names that would become the same Rust type.
///
/// `fooBar`, `foo-bar` and `foo_bar` all render as `FooBar`. Without this the generator emits two
/// definitions of one name and the failure surfaces as a compile error in generated code, which
/// points at the output instead of at the specification that caused it.
fn check_names_do_not_collide(schemas: &BTreeMap<String, Schema>) -> Result<(), Error> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();

    for name in schemas.keys() {
        let rust = type_name(name);

        if rust.is_empty() {
            return Err(Error::Unsupported {
                what: format!("schema name `{name}` has no usable characters for a Rust type"),
            });
        }

        if let Some(first) = seen.insert(rust.clone(), name) {
            return Err(Error::Unsupported {
                what: format!(
                    "schemas `{first}` and `{name}` both become the Rust type `{rust}`; rename one"
                ),
            });
        }
    }

    Ok(())
}

fn emit_named(name: &str, schema: &Schema, out: &mut String) -> Result<(), Error> {
    let (kind, _) = schema.resolve_kind().map_err(|reason| Error::Unsupported {
        what: format!("schema `{name}` uses {reason}"),
    })?;

    require_description(schema.description.as_deref(), &format!("schema `{name}`"))?;

    match (kind.as_deref(), schema.values.as_ref()) {
        (Some("string"), Some(values)) => emit_enum(name, schema, values, out),
        (Some("object"), None) => emit_struct(name, schema, out),
        _ => Err(Error::Unsupported {
            what: format!(
                "schema `{name}`: only string enums and objects with `properties` can be named \
                 types"
            ),
        }),
    }
}

/// Refuses an undocumented part of the contract.
///
/// The specification is what the published documentation is built from and what a client author
/// reads. An undescribed field there is an undescribed field everywhere, so it is a generator
/// error rather than a lint somebody silences.
pub fn require_description(description: Option<&str>, what: &str) -> Result<(), Error> {
    match description.map(str::trim) {
        Some(text) if !text.is_empty() => Ok(()),
        _ => Err(Error::Unsupported {
            what: format!("{what} has no `description`; every part of the contract needs one"),
        }),
    }
}

fn emit_enum(
    name: &str,
    schema: &Schema,
    values: &[String],
    out: &mut String,
) -> Result<(), Error> {
    docs(schema.description.as_deref(), 0, out);
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\n");
    let _ = writeln!(out, "pub enum {name} {{");

    let mut seen: BTreeMap<String, &str> = BTreeMap::new();

    for value in values {
        let variant = type_name(value);
        if variant.is_empty() {
            return Err(Error::Unsupported {
                what: format!(
                    "schema `{name}` has enum value `{value}`, which has no usable characters for \
                     a Rust variant"
                ),
            });
        }
        if let Some(first) = seen.insert(variant.clone(), value) {
            return Err(Error::Unsupported {
                what: format!(
                    "schema `{name}`: enum values `{first}` and `{value}` both become the variant \
                     `{variant}`; rename one"
                ),
            });
        }

        let _ = writeln!(out, "    /// `{value}`");
        let _ = writeln!(out, "    #[serde(rename = \"{}\")]", string_literal(value));
        let _ = writeln!(out, "    {variant},");
    }

    out.push_str("}\n\n");
    Ok(())
}

/// Rust keywords that would be rejected in field position.
///
/// A property genuinely called `type` or `match` is ordinary in JSON, so it is emitted as a raw
/// identifier rather than refused. Serde uses the identifier without the `r#`, so the wire name is
/// unchanged and no rename is needed.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "become", "box", "break", "const", "continue", "do", "dyn", "else",
    "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let", "loop", "macro",
    "match", "mod", "move", "mut", "override", "priv", "ref", "return", "static", "struct",
    "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use", "virtual", "where",
    "while", "yield",
];

/// Renders a property name as a Rust field identifier.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] when the name is not a usable identifier at all. Quietly
/// renaming it is not an option: the name *is* the wire format the specification promised.
pub fn field_ident(name: &str, context: &str) -> Result<String, Error> {
    let mut chars = name.chars();
    let usable = match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    };

    if !usable {
        return Err(Error::Unsupported {
            what: format!(
                "{context}: `{name}` is not a usable Rust field name; use letters, digits and \
                 underscores, starting with a letter or underscore"
            ),
        });
    }

    // These cannot be raw identifiers either, so there is no escape hatch for them.
    if matches!(name, "self" | "Self" | "crate" | "super") {
        return Err(Error::Unsupported {
            what: format!("{context}: `{name}` cannot be a Rust field name under any spelling"),
        });
    }

    if KEYWORDS.contains(&name) {
        return Ok(format!("r#{name}"));
    }

    Ok(name.to_owned())
}

fn emit_struct(name: &str, schema: &Schema, out: &mut String) -> Result<(), Error> {
    if schema.properties.is_empty() {
        return Err(Error::Unsupported {
            what: format!("schema `{name}` is an object with no `properties`"),
        });
    }

    docs(schema.description.as_deref(), 0, out);
    out.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n");
    let _ = writeln!(out, "pub struct {name} {{");

    for (field, property) in &schema.properties {
        require_description(
            property.description.as_deref(),
            &format!("`{name}.{field}`"),
        )?;

        let required = schema.required.iter().any(|r| r == field);
        let rust = rust_type(property, &format!("{name}.{field}"))?;
        let optional = !required || rust.nullable;
        let ty = if optional {
            format!("Option<{}>", rust.ty)
        } else {
            rust.ty
        };

        docs(property.description.as_deref(), 4, out);
        if optional {
            out.push_str("    #[serde(default)]\n");
        }
        let ident = field_ident(field, &format!("schema `{name}`"))?;
        let _ = writeln!(out, "    pub {ident}: {ty},");
    }

    out.push_str("}\n\n");
    Ok(())
}

/// A Rust type, plus whether the schema permitted `null`.
pub struct RustType {
    /// The rendered type, without any `Option` wrapper.
    pub ty: String,
    /// Whether `null` was one of the permitted types.
    pub nullable: bool,
}

/// Maps a schema to a Rust type.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] for any construct the generator cannot express, naming `context`
/// so the offending part of the spec can be found.
pub fn rust_type(schema: &Schema, context: &str) -> Result<RustType, Error> {
    if let Some(reference) = &schema.reference {
        let name = reference.rsplit('/').next().unwrap_or_default();
        if name.is_empty() {
            return Err(Error::Unsupported {
                what: format!("{context}: could not read `$ref` `{reference}`"),
            });
        }
        return Ok(RustType {
            ty: name.to_owned(),
            nullable: false,
        });
    }

    let (kind, nullable) = schema.resolve_kind().map_err(|reason| Error::Unsupported {
        what: format!("{context} uses {reason}"),
    })?;

    let Some(kind) = kind else {
        return Err(Error::Unsupported {
            what: format!("{context} has neither `type` nor `$ref`"),
        });
    };

    let ty = match kind.as_str() {
        "string" => {
            // An inline `enum` would be silently widened to `String`, quietly dropping the
            // constraint the specification promised. Naming it makes it a real Rust enum.
            if schema.values.is_some() {
                return Err(Error::Unsupported {
                    what: format!(
                        "{context} has an inline `enum`; give it a name under \
                         `components/schemas` and `$ref` it, so the constraint survives"
                    ),
                });
            }
            "String".to_owned()
        }
        "boolean" => "bool".to_owned(),
        "integer" => match schema.format.as_deref() {
            Some("int32") | None => "i32".to_owned(),
            Some("int64") => "i64".to_owned(),
            Some(other) => {
                return Err(Error::Unsupported {
                    what: format!("{context} uses integer format `{other}`"),
                });
            }
        },
        "number" => match schema.format.as_deref() {
            Some("double" | "float") | None => "f64".to_owned(),
            Some(other) => {
                return Err(Error::Unsupported {
                    what: format!("{context} uses number format `{other}`"),
                });
            }
        },
        "array" => {
            let Some(items) = &schema.items else {
                return Err(Error::Unsupported {
                    what: format!("{context} is an array with no `items`"),
                });
            };
            let inner = rust_type(items, &format!("{context}[]"))?;
            format!("Vec<{}>", inner.ty)
        }
        "object" => {
            let Some(values) = &schema.additional_properties else {
                return Err(Error::Unsupported {
                    what: format!(
                        "{context} is an inline object; give it a name under \
                         `components/schemas` or use `additionalProperties`"
                    ),
                });
            };
            let inner = rust_type(values, &format!("{context}{{}}"))?;
            format!("std::collections::BTreeMap<String, {}>", inner.ty)
        }
        other => {
            return Err(Error::Unsupported {
                what: format!("{context} uses type `{other}`"),
            });
        }
    };

    Ok(RustType { ty, nullable })
}
