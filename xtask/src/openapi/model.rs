//! The subset of `OpenAPI` that Layover's generator understands.
//!
//! This is deliberately partial. Every construct the generator cannot turn into Rust is an error
//! naming the construct, so `api/openapi.yaml` cannot quietly grow something the server does not
//! implement. Widening the subset is a change to this file, reviewed like any other.
//!
//! Several fields here are never read. They are not dead: `deny_unknown_fields` means a field has
//! to be declared to be *accepted*, so removing them would make a valid specification fail to
//! parse. Declaring them is how the parser stays strict about genuine typos.
#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::Deserialize;

/// A whole `OpenAPI` document.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Specification version, such as `3.2.0`.
    pub openapi: String,
    /// Document metadata.
    pub info: Info,
    /// Declared servers; carried through but not used by the generator.
    #[serde(default)]
    pub servers: Vec<serde_yaml_bw::Value>,
    /// Declared tags; carried through but not used by the generator.
    #[serde(default)]
    pub tags: Vec<serde_yaml_bw::Value>,
    /// Operations, keyed by path template.
    #[serde(default)]
    pub paths: BTreeMap<String, PathItem>,
    /// Reusable schemas.
    #[serde(default)]
    pub components: Components,
}

/// Document metadata.
#[derive(Debug, Deserialize)]
pub struct Info {
    /// Human-readable API name.
    pub title: String,
    /// API version.
    pub version: String,
    /// One-line summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Longer description.
    #[serde(default)]
    pub description: Option<String>,
    /// Licence metadata.
    #[serde(default)]
    pub license: Option<serde_yaml_bw::Value>,
}

/// Reusable components.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Components {
    /// Named schemas.
    #[serde(default)]
    pub schemas: BTreeMap<String, Schema>,
}

/// The operations available at one path.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathItem {
    /// `GET`.
    #[serde(default)]
    pub get: Option<Operation>,
    /// `PUT`.
    #[serde(default)]
    pub put: Option<Operation>,
    /// `POST`.
    #[serde(default)]
    pub post: Option<Operation>,
    /// `DELETE`.
    #[serde(default)]
    pub delete: Option<Operation>,
    /// `PATCH`.
    #[serde(default)]
    pub patch: Option<Operation>,
}

impl PathItem {
    /// Returns each declared operation with its HTTP method, in a stable order.
    pub fn operations(&self) -> Vec<(&'static str, &Operation)> {
        [
            ("get", self.get.as_ref()),
            ("put", self.put.as_ref()),
            ("post", self.post.as_ref()),
            ("delete", self.delete.as_ref()),
            ("patch", self.patch.as_ref()),
        ]
        .into_iter()
        .filter_map(|(method, operation)| operation.map(|operation| (method, operation)))
        .collect()
    }
}

/// One operation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
// `operation_id` has to keep that name: `rename_all = "camelCase"` maps it to `operationId`,
// which is what OpenAPI calls it.
#[allow(clippy::struct_field_names)]
pub struct Operation {
    /// Unique identifier, turned into a Rust method name.
    pub operation_id: String,
    /// One-line summary, emitted as a doc comment.
    #[serde(default)]
    pub summary: Option<String>,
    /// Longer description, emitted as a doc comment.
    #[serde(default)]
    pub description: Option<String>,
    /// Grouping tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Path and query parameters.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Request body, if any.
    #[serde(default)]
    pub request_body: Option<RequestBody>,
    /// Responses, keyed by status code.
    #[serde(default)]
    pub responses: BTreeMap<String, Response>,
}

/// A path or query parameter.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Where it appears: `path` or `query`.
    #[serde(rename = "in")]
    pub location: String,
    /// Whether it must be supplied.
    #[serde(default)]
    pub required: bool,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// The parameter's type.
    pub schema: Schema,
}

/// A request body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestBody {
    /// Whether the body must be supplied.
    #[serde(default)]
    pub required: bool,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Bodies by media type.
    pub content: BTreeMap<String, MediaType>,
}

/// One response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// What this response means.
    pub description: String,
    /// Bodies by media type.
    #[serde(default)]
    pub content: BTreeMap<String, MediaType>,
}

/// A body of one media type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaType {
    /// The body's schema.
    pub schema: Schema,
}

/// A JSON Schema, restricted to what the generator can emit.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Schema {
    /// A reference to a named schema.
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
    /// The JSON type, or a list of types including `null`.
    #[serde(rename = "type", default)]
    pub kind: Option<TypeSpec>,
    /// Format hint, such as `int64` or `date-time`.
    #[serde(default)]
    pub format: Option<String>,
    /// Permitted values, which turn a named string schema into an enum.
    #[serde(rename = "enum", default)]
    pub values: Option<Vec<String>>,
    /// Human-readable description, emitted as a doc comment.
    #[serde(default)]
    pub description: Option<String>,
    /// Object properties.
    #[serde(default)]
    pub properties: BTreeMap<String, Schema>,
    /// Which properties must be present.
    #[serde(default)]
    pub required: Vec<String>,
    /// Element schema, for arrays.
    #[serde(default)]
    pub items: Option<Box<Schema>>,
    /// Value schema, for maps.
    #[serde(default)]
    pub additional_properties: Option<Box<Schema>>,
}

impl Schema {
    /// Returns the single non-null type name, and whether `null` was also permitted.
    ///
    /// # Errors
    ///
    /// Returns a description of the problem when the type list is empty of concrete types or
    /// names several of them, neither of which the generator can express.
    pub fn resolve_kind(&self) -> Result<(Option<String>, bool), String> {
        let Some(spec) = &self.kind else {
            return Ok((None, false));
        };

        let names: Vec<String> = match spec {
            TypeSpec::One(name) => vec![name.clone()],
            TypeSpec::Many(names) => names.clone(),
        };

        let nullable = names.iter().any(|name| name == "null");
        let mut concrete: Vec<String> = names.into_iter().filter(|name| name != "null").collect();

        match concrete.len() {
            1 => Ok((Some(concrete.remove(0)), nullable)),
            0 => Err("a schema whose only type is `null`".to_owned()),
            _ => Err(format!("a union of several types: {concrete:?}")),
        }
    }
}

/// `type: string` or `type: [string, "null"]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TypeSpec {
    /// A single type name.
    One(String),
    /// Several type names, at most one of which is concrete.
    Many(Vec<String>),
}
