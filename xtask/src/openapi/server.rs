//! Turning `OpenAPI` operations into an `Api` trait and an axum router.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::model::{Document, Operation};
use super::schema::rust_type;
use super::{Error, docs, method_name, string_literal, type_name};

/// Everything the generator needs to know about one operation.
struct Resolved<'a> {
    method: &'static str,
    path: &'a str,
    operation: &'a Operation,
    /// Rust method name.
    name: String,
    /// Type prefix for generated parameter structs.
    prefix: String,
    path_params: Vec<&'a super::model::Parameter>,
    query_params: Vec<&'a super::model::Parameter>,
    body: Option<String>,
    success_status: u16,
    success: Success,
}

/// What an operation returns on success.
enum Success {
    Json(String),
    EventStream,
}

/// Emits the `Api` trait, its parameter structs and the axum router.
pub fn emit_server(document: &Document, out: &mut String) -> Result<(), Error> {
    let operations = resolve(document)?;

    for resolved in &operations {
        emit_params(resolved, out)?;
    }

    emit_trait(&operations, out);
    emit_router(&operations, out)?;
    emit_operation_table(&operations, out);

    Ok(())
}

fn resolve(document: &Document) -> Result<Vec<Resolved<'_>>, Error> {
    let mut resolved = Vec::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();

    for (path, item) in &document.paths {
        for (method, operation) in item.operations() {
            let id = &operation.operation_id;

            // `listRuns` and `list-runs` would both become `list_runs`, emitting two trait methods
            // and two handlers with one name. Catch it here, where the specification can be named,
            // rather than as a compile error in generated code.
            let name = method_name(id);
            if name.is_empty() {
                return Err(Error::Unsupported {
                    what: format!("operationId `{id}` has no usable characters for a method name"),
                });
            }
            if let Some(first) = names.insert(name.clone(), id.clone()) {
                return Err(Error::Unsupported {
                    what: format!(
                        "operations `{first}` and `{id}` both become the method `{name}`; \
                         rename one"
                    ),
                });
            }

            let mut path_params = Vec::new();
            let mut query_params = Vec::new();

            for parameter in &operation.parameters {
                match parameter.location.as_str() {
                    "path" => path_params.push(parameter),
                    "query" => query_params.push(parameter),
                    other => {
                        return Err(Error::Unsupported {
                            what: format!("operation `{id}` has a parameter in `{other}`"),
                        });
                    }
                }
            }

            let body = match &operation.request_body {
                None => None,
                Some(request) => {
                    if !request.required {
                        return Err(Error::Unsupported {
                            what: format!(
                                "operation `{id}` has an optional request body; the generator \
                                 emits a mandatory extractor, which would reject a request the \
                                 specification allows"
                            ),
                        });
                    }
                    let Some(media) = request.content.get("application/json") else {
                        return Err(Error::Unsupported {
                            what: format!(
                                "operation `{id}` has a request body that is not application/json"
                            ),
                        });
                    };
                    Some(rust_type(&media.schema, &format!("{id} request body"))?.ty)
                }
            };

            let (success_status, success) = success_of(operation)?;

            resolved.push(Resolved {
                method,
                path,
                operation,
                name,
                prefix: type_name(id),
                path_params,
                query_params,
                body,
                success_status,
                success,
            });
        }
    }

    Ok(resolved)
}

fn success_of(operation: &Operation) -> Result<(u16, Success), Error> {
    let id = &operation.operation_id;

    let (code, response) = operation
        .responses
        .iter()
        .filter_map(|(code, response)| code.parse::<u16>().ok().map(|code| (code, response)))
        .filter(|(code, _)| (200..300).contains(code))
        .min_by_key(|(code, _)| *code)
        .ok_or_else(|| Error::Unsupported {
            what: format!("operation `{id}` declares no 2xx response"),
        })?;

    if response.content.contains_key("text/event-stream") {
        return Ok((code, Success::EventStream));
    }

    let Some(media) = response.content.get("application/json") else {
        return Err(Error::Unsupported {
            what: format!(
                "operation `{id}` has a {code} response that is neither application/json nor \
                 text/event-stream"
            ),
        });
    };

    Ok((
        code,
        Success::Json(rust_type(&media.schema, &format!("{id} response"))?.ty),
    ))
}

fn emit_params(resolved: &Resolved<'_>, out: &mut String) -> Result<(), Error> {
    for (suffix, params) in [
        ("Path", &resolved.path_params),
        ("Query", &resolved.query_params),
    ] {
        if params.is_empty() {
            continue;
        }

        let name = format!("{}{suffix}", resolved.prefix);
        let _ = writeln!(
            out,
            "/// {} parameters for `{}`.",
            suffix.to_lowercase(),
            resolved.operation.operation_id
        );
        out.push_str("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n");
        let _ = writeln!(out, "pub struct {name} {{");

        for parameter in &**params {
            super::schema::require_description(
                parameter.description.as_deref(),
                &format!(
                    "parameter `{}` of `{}`",
                    parameter.name, resolved.operation.operation_id
                ),
            )?;

            let rust = rust_type(
                &parameter.schema,
                &format!(
                    "{} parameter `{}`",
                    resolved.operation.operation_id, parameter.name
                ),
            )?;
            let optional = !parameter.required || rust.nullable;
            let ty = if optional {
                format!("Option<{}>", rust.ty)
            } else {
                rust.ty
            };

            docs(parameter.description.as_deref(), 4, out);
            if optional {
                out.push_str("    #[serde(default)]\n");
            }
            let ident = super::schema::field_ident(
                &parameter.name,
                &format!("operation `{}`", resolved.operation.operation_id),
            )?;
            let _ = writeln!(out, "    pub {ident}: {ty},");
        }

        out.push_str("}\n\n");
    }

    Ok(())
}

fn signature(resolved: &Resolved<'_>) -> String {
    let mut args = String::from("&self");

    if !resolved.path_params.is_empty() {
        let _ = write!(args, ", path: {}Path", resolved.prefix);
    }
    if !resolved.query_params.is_empty() {
        let _ = write!(args, ", query: {}Query", resolved.prefix);
    }
    if let Some(body) = &resolved.body {
        let _ = write!(args, ", body: {body}");
    }

    let returns = match &resolved.success {
        Success::Json(ty) => ty.clone(),
        Success::EventStream => "EventStream".to_owned(),
    };

    // Written as an explicit `impl Future + Send` rather than `async fn`, because axum handlers
    // require Send futures and a bare `async fn` in a trait does not promise one. Implementors may
    // still write `async fn`.
    format!(
        "fn {}({args}) -> impl core::future::Future<Output = Result<{returns}, Problem>> + Send",
        resolved.name
    )
}

fn emit_trait(operations: &[Resolved<'_>], out: &mut String) {
    out.push_str(
        "/// Everything the Tower must implement to serve this API.\n\
         ///\n\
         /// One method per `operationId`. Adding an operation to `api/openapi.yaml` adds a method\n\
         /// here, so an unimplemented endpoint is a compile error rather than a 404 discovered in\n\
         /// production.\n",
    );
    out.push_str("pub trait Api: Send + Sync + 'static {\n");

    for resolved in operations {
        docs(resolved.operation.summary.as_deref(), 4, out);
        if resolved.operation.description.is_some() {
            out.push_str("    ///\n");
            docs(resolved.operation.description.as_deref(), 4, out);
        }
        out.push_str("    ///\n");
        let _ = writeln!(
            out,
            "    /// `{} {}`",
            resolved.method.to_uppercase(),
            resolved.path
        );
        let _ = writeln!(out, "    {};", signature(resolved));
    }

    out.push_str("}\n\n");
}

fn emit_router(operations: &[Resolved<'_>], out: &mut String) -> Result<(), Error> {
    out.push_str(
        "/// Builds the axum router for this API.\n\
         ///\n\
         /// Routes come straight from the specification, so a path can only exist here if it\n\
         /// exists there.\n",
    );
    out.push_str("pub fn router<A: Api>(api: std::sync::Arc<A>) -> axum::Router {\n");
    out.push_str("    axum::Router::new()\n");

    // axum panics when the same path is registered twice, so every method on a path has to be
    // chained into a single `.route()` call.
    let mut paths: Vec<&str> = operations.iter().map(|resolved| resolved.path).collect();
    paths.dedup();

    for path in paths {
        // The first method builds a MethodRouter; the rest are chained onto it, which is why only
        // the first is written with its full path.
        let mut handlers = String::new();
        for (index, resolved) in operations
            .iter()
            .filter(|resolved| resolved.path == path)
            .enumerate()
        {
            if index == 0 {
                let _ = write!(handlers, "axum::routing::{}(", resolved.method);
            } else {
                let _ = write!(handlers, ".{}(", resolved.method);
            }
            let _ = write!(handlers, "handle_{}::<A>)", resolved.name);
        }

        let _ = writeln!(
            out,
            "        .route(\"{}\", {handlers})",
            string_literal(path)
        );
    }

    out.push_str("        .with_state(api)\n}\n\n");

    for resolved in operations {
        emit_handler(resolved, out)?;
    }

    Ok(())
}

/// Maps a status code to an axum constant, so the generated code cannot fail at runtime.
fn status_constant(code: u16) -> Result<&'static str, Error> {
    match code {
        200 => Ok("OK"),
        201 => Ok("CREATED"),
        202 => Ok("ACCEPTED"),
        204 => Ok("NO_CONTENT"),
        other => Err(Error::Unsupported {
            what: format!("success status {other}; add it to `status_constant`"),
        }),
    }
}

fn emit_handler(resolved: &Resolved<'_>, out: &mut String) -> Result<(), Error> {
    let mut extractors =
        String::from("    axum::extract::State(api): axum::extract::State<std::sync::Arc<A>>,\n");
    let mut call_args = String::new();

    if !resolved.path_params.is_empty() {
        let _ = writeln!(
            extractors,
            "    axum::extract::Path(path): axum::extract::Path<{}Path>,",
            resolved.prefix
        );
        call_args.push_str("path, ");
    }
    if !resolved.query_params.is_empty() {
        let _ = writeln!(
            extractors,
            "    axum::extract::Query(query): axum::extract::Query<{}Query>,",
            resolved.prefix
        );
        call_args.push_str("query, ");
    }
    // The body extractor consumes the request, so axum requires it last.
    if let Some(body) = &resolved.body {
        let _ = writeln!(extractors, "    axum::Json(body): axum::Json<{body}>,");
        call_args.push_str("body, ");
    }

    let call_args = call_args.trim_end_matches(", ");

    let _ = writeln!(
        out,
        "async fn handle_{}<A: Api>(\n{extractors}) -> axum::response::Response {{",
        resolved.name
    );
    let _ = writeln!(out, "    match api.{}({call_args}).await {{", resolved.name);

    match &resolved.success {
        Success::Json(_) => {
            let _ = writeln!(
                out,
                "        Ok(value) => (axum::http::StatusCode::{}, axum::Json(value)).into_response(),",
                status_constant(resolved.success_status)?
            );
        }
        Success::EventStream => {
            out.push_str("        Ok(stream) => stream.into_response(),\n");
        }
    }

    out.push_str("        Err(problem) => problem.into_response(),\n    }\n}\n\n");
    Ok(())
}

fn emit_operation_table(operations: &[Resolved<'_>], out: &mut String) {
    out.push_str(
        "/// Every operation the specification declares, as (method, path, operationId).\n\
         ///\n\
         /// Exposed so tests can assert the router and the specification agree.\n",
    );
    let _ = writeln!(
        out,
        "pub const OPERATIONS: [(&str, &str, &str); {}] = [",
        operations.len()
    );

    for resolved in operations {
        let _ = writeln!(
            out,
            "    (\"{}\", \"{}\", \"{}\"),",
            resolved.method.to_uppercase(),
            string_literal(resolved.path),
            string_literal(&resolved.operation.operation_id)
        );
    }

    out.push_str("];\n");
}
